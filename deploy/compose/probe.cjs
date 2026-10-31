#!/usr/bin/env node
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const https = require('node:https');
const tls = require('node:tls');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');

const MAX_BYTES = 4 * 1024 * 1024;
const read = filename => {
  const stat = fs.lstatSync(filename);
  assert(stat.isFile() && !stat.isSymbolicLink() && stat.nlink === 1 && stat.size <= MAX_BYTES, 'unsafe probe input');
  return fs.readFileSync(filename);
};
const json = filename => JSON.parse(read(filename).toString('utf8').replace(/^\uFEFF/, ''));
function save(filename, value) {
  const temporary = `${filename}.${crypto.randomUUID()}.tmp`;
  fs.writeFileSync(temporary, JSON.stringify(value, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
  fs.renameSync(temporary, filename);
}

async function verify(root) {
  assert(Number(process.versions.node.split('.')[0]) >= 22, 'Node 22 or newer is required');
  assert(path.isAbsolute(root), 'absolute deployment directory required');
  let cursor = root;
  for (;;) {
    const stat = fs.lstatSync(cursor);
    assert(stat.isDirectory() && !stat.isSymbolicLink(), 'linked deployment path refused');
    const parent = path.dirname(cursor);
    if (parent === cursor) break;
    cursor = parent;
  }
  const manifest = json(path.join(root, 'manifest.json'));
  assert.equal(manifest.purpose, 'ColossusCompose');
  assert.equal(path.resolve(manifest.root), path.resolve(root));
  assert.equal(manifest.initialized, true);
  const origin = `https://${manifest.hostname}:${manifest.browserPort}`;
  const ca = read(path.join(root, 'browser-ca.pem'));
  const receipt = { passed: false, deploymentId: manifest.deploymentId, origin, checks: [], profile: manifest.profile };
  const receiptFile = path.join(root, 'verification.json');
  let cookie;
  const request = (method, route, body, statuses = [200], stream = false) => new Promise((resolve, reject) => {
    const payload = body === undefined ? undefined : Buffer.from(JSON.stringify(body));
    const req = https.request(origin + route, {
      method, ca, rejectUnauthorized: true, agent: false,
      headers: { Origin: origin, 'X-Colossus-CSRF': '1', ...(cookie ? { Cookie: cookie } : {}),
        ...(payload ? { 'Content-Type': 'application/json', 'Content-Length': payload.length } : {}) },
    }, response => {
      if (!statuses.includes(response.statusCode)) { response.resume(); reject(new Error(`HTTP ${method} ${route}: ${response.statusCode}`)); return; }
      if (method === 'POST' && route === '/auth/local') {
        const cookies = response.headers['set-cookie'] ?? [];
        cookie = cookies.find(value => value.startsWith('colossus_session='))?.split(';')[0];
        assert(cookie, 'secure sign-in cookie missing');
      }
      let bytes = 0;
      const chunks = [];
      response.on('data', chunk => {
        bytes += chunk.length;
        if (bytes > MAX_BYTES) { req.destroy(new Error('response bound exceeded')); return; }
        chunks.push(chunk);
        if (stream && Buffer.concat(chunks).toString('utf8').includes('\n\n')) {
          assert.match(response.headers['content-type'], /^text\/event-stream/);
          resolve(Buffer.concat(chunks).toString('utf8'));
          req.destroy();
        }
      });
      response.on('error', reject);
      response.on('end', () => resolve({ status: response.statusCode, text: Buffer.concat(chunks).toString('utf8') }));
    });
    req.setTimeout(30000, () => req.destroy(new Error('request timed out')));
    req.on('error', reject);
    req.end(payload);
  });
  const api = async (method, route, body, statuses) => JSON.parse((await request(method, route, body, statuses)).text);
  try {
    await request('GET', '/health/ready');
    assert.match((await request('GET', '/')).text, /<html/i);
    assert.match((await request('GET', '/docs/')).text, /<html/i);
    const search = await api('GET', '/docs/search.json');
    assert(search.items.length > 100, 'documentation search is incomplete');
    for (const item of search.items) {
      const url = new URL(item.location, origin + '/docs/');
      assert.equal(url.origin, origin);
      assert(url.pathname.startsWith('/docs/'), 'documentation search escapes the deployment');
    }
    receipt.checks.push('https-and-database-readiness', 'web', 'documentation-and-local-search');
    await request('GET', '/api/me', undefined, [403]);
    await request('POST', '/auth/local', { username: 'admin', password: read(path.join(root, 'material/controller/admin-password')).toString('utf8') }, [200,204]);
    const identity = await api('GET', '/api/me');
    assert.equal(identity.user.is_admin, true);
    receipt.adminId = identity.user.id;
    receipt.checks.push('anonymous-access-denied', 'administrator-login');
    await new Promise((resolve, reject) => {
      const expected = new crypto.X509Certificate(read(path.join(root, 'material/controller/server.pem'))).fingerprint256;
      let verified = false;
      let required = false;
      const socket = tls.connect({ host: manifest.hostname, port: manifest.connectorPort, servername: manifest.hostname,
        ALPNProtocols: ['h2'], ca: read(path.join(root, 'connector-ca.pem')), rejectUnauthorized: true });
      const timer = setTimeout(() => socket.destroy(new Error('connector TLS timeout')), 10000);
      socket.on('secureConnect', () => {
        verified = socket.authorized && socket.alpnProtocol === 'h2' && socket.getPeerCertificate().fingerprint256 === expected;
        socket.write('PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n');
      });
      socket.on('error', error => { if (/certificate required/i.test(error.message)) required = true; else reject(error); });
      socket.on('data', () => socket.end());
      socket.on('close', () => { clearTimeout(timer); if (verified && required) resolve(); else reject(new Error('connector mTLS boundary was not verified')); });
    });
    receipt.checks.push('connector-certificate-and-http2', 'connector-client-certificate-required');
    if (manifest.profile === 'hosted-demo') {
      const status = await api('GET', '/api/projects/local/chat');
      assert.equal(status.enabled, true);
      const settings = await api('GET', '/api/projects/local/chat/settings');
      // Never turn an operator's later provider configuration into a paid test.
      assert.equal(settings.providers.profiles.main.kind, 'echo', 'Hosted probe only runs against the offline Echo provider');
      const intentFile = path.join(root, 'verification-intent.json');
      let intent;
      if (fs.existsSync(intentFile)) {
        intent = json(intentFile);
        assert(intent.threadId && intent.taskId, 'prior probe POST outcome is unresolved; it will not be repeated');
      } else {
        intent = { marker: 'Compose acceptance ' + crypto.randomUUID(), key: crypto.randomUUID() };
        save(intentFile, intent);
        const created = await api('POST', '/api/projects/local/chat/threads', { request: {
          input: [{ text: intent.marker }], mode: 'execute', role: 'primary', model_profile: 'main',
          reasoning_effort: null, plugin_skill_ids: [], session_id: null, end_user_id: null,
          goal_max_iterations: 0, research_depth: null, research_sources: [], plan_action: null,
          branch: null, max_turns: 2, idempotency_key: intent.key,
        } });
        intent.threadId = created.thread.thread_id;
        intent.taskId = created.task.task_id;
        save(intentFile, intent);
      }
      const taskPath = `/api/projects/local/tasks/${encodeURIComponent(intent.taskId)}`;
      const stream = await request('GET', taskPath + '/events', undefined, [200], true);
      assert.match(stream, /data:/);
      const deadline = Date.now() + 60000;
      let task;
      do {
        task = (await api('GET', taskPath)).task;
        assert(!task.dispatch_error, 'Hosted dispatch failed');
        if (task.snapshot?.run?.status === 'completed') break;
        assert(!['failed','interrupted','outcome_unknown','cancelled'].includes(task.snapshot?.run?.status), 'Hosted run did not complete');
        await new Promise(resolve => setTimeout(resolve, 250));
      } while (Date.now() < deadline);
      assert.equal(task.snapshot?.run?.status, 'completed', 'Hosted run timed out');
      const detail = await api('GET', `/api/projects/local/threads/${encodeURIComponent(intent.threadId)}`);
      assert(detail.messages.some(message => message.role === 'assistant' && message.text.includes(intent.marker)),
        'retained conversation did not contain the Echo assistant response');
      receipt.threadId = intent.threadId;
      receipt.taskId = intent.taskId;
      receipt.checks.push('hosted-offline-echo', 'sse-through-https-proxy', 'retained-conversation');
    }
    receipt.passed = true;
    receipt.verifiedAtUtc = new Date().toISOString();
    save(receiptFile, receipt);
    process.stdout.write(`Compose acceptance passed: ${receipt.checks.join(', ')}.\n`);
    return receipt;
  } catch (error) {
    receipt.error = error.message;
    save(receiptFile, receipt);
    throw error;
  }
}
if (require.main === module) verify(path.resolve(process.argv[2] ?? '')).catch(error => { process.stderr.write(error.message + '\n'); process.exitCode = 1; });
module.exports = { verify };

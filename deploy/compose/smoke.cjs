// Acceptance for the default demo: run Echo and reuse it after restart.
const fs = require('node:fs');
const path = require('node:path');
const https = require('node:https');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');

async function smoke(directory = __dirname) {
  const root = path.join(directory, '.local');
  const read = file => fs.readFileSync(path.join(root, file), 'utf8');
  const config = JSON.parse(read('material/controller/config.json'));
  const origin = config.public_origin;
  const ca = read('material/browser/ca.pem');
  let cookie;
  const request = (method, route, body, expected = [200], streaming = false) => new Promise((resolve, reject) => {
    const payload = body === undefined ? undefined : Buffer.from(JSON.stringify(body));
    const req = https.request(origin + route, {
      method, ca, rejectUnauthorized: true, agent: false,
      headers: {Origin: origin, 'X-Colossus-CSRF': '1', ...(cookie ? {Cookie: cookie} : {}),
        ...(payload ? {'Content-Type': 'application/json', 'Content-Length': payload.length} : {})},
    }, response => {
      if (!expected.includes(response.statusCode)) {response.resume(); reject(new Error(`${method} ${route}: HTTP ${response.statusCode}`)); return;}
      if (route === '/auth/local') {
        const session = (response.headers['set-cookie'] ?? []).find(value => value.startsWith('colossus_session='));
        assert(session && /;\s*Secure/i.test(session), 'secure session cookie missing');
        cookie = session.split(';')[0];
      }
      let text = '';
      response.setEncoding('utf8');
      response.on('data', chunk => {
        text += chunk;
        if (Buffer.byteLength(text) > 2 * 1024 * 1024) {req.destroy(new Error('response exceeded bound')); return;}
        if (streaming && text.includes('\n\n')) {
          assert.match(response.headers['content-type'], /^text\/event-stream/);
          resolve(text); req.destroy();
        }
      });
      response.on('error', reject);
      response.on('end', () => resolve(text));
    });
    req.setTimeout(30000, () => req.destroy(new Error('request timeout')));
    req.on('error', reject); req.end(payload);
  });
  const api = async (...args) => JSON.parse(await request(...args));
  await request('GET', '/health/ready');
  assert.match(await request('GET', '/'), /<html/i);
  assert.match(await request('GET', '/docs/'), /<html/i);
  await request('GET', '/api/me', undefined, [403]);
  await request('POST', '/auth/local', {username: 'admin', password: read('material/controller/admin-password')}, [200, 204]);
  const identity = (await api('GET', '/api/me')).user;
  assert(identity.is_admin);
  assert.equal((await api('GET', '/api/projects/local')).project.description, 'Colossus Compose default project');
  const settings = await api('GET', '/api/projects/local/chat/settings');
  assert.equal(settings.providers.profiles.main.kind, 'echo', 'smoke only uses offline Echo');
  const receipt = path.join(root, 'smoke.json');
  let intent;
  if (fs.existsSync(receipt)) {
    intent = JSON.parse(read('smoke.json'));
    assert(intent.threadId && intent.taskId, 'previous POST outcome unresolved; not repeated');
  } else {
    intent = {marker: 'Compose smoke ' + crypto.randomUUID(), key: crypto.randomUUID()};
    fs.writeFileSync(receipt, JSON.stringify(intent), {flag: 'wx', mode: 0o600});
    const created = await api('POST', '/api/projects/local/chat/threads', {request: {
      input: [{text: intent.marker}], mode: 'execute', role: 'primary', model_profile: 'main',
      reasoning_effort: null, plugin_skill_ids: [], session_id: null, end_user_id: null,
      goal_max_iterations: 0, research_depth: null, research_sources: [], plan_action: null,
      branch: null, max_turns: 2, idempotency_key: intent.key,
    }});
    intent.threadId = created.thread.thread_id; intent.taskId = created.task.task_id;
    fs.writeFileSync(receipt, JSON.stringify(intent), {mode: 0o600});
  }
  const taskPath = `/api/projects/local/tasks/${encodeURIComponent(intent.taskId)}`;
  assert.match(await request('GET', taskPath + '/events', undefined, [200], true), /data:/);
  const deadline = Date.now() + 60000;
  let status;
  do {
    const task = (await api('GET', taskPath)).task;
    assert(!task.dispatch_error, 'Chat dispatch failed');
    status = task.snapshot?.run?.status;
    if (status === 'completed') break;
    assert(!['failed', 'interrupted', 'outcome_unknown', 'cancelled'].includes(status), 'Chat failed');
    await new Promise(resolve => setTimeout(resolve, 250));
  } while (Date.now() < deadline);
  assert.equal(status, 'completed', 'Chat timeout');
  const detail = await api('GET', `/api/projects/local/threads/${encodeURIComponent(intent.threadId)}`);
  assert(detail.messages.some(message => message.role === 'assistant' && message.text.includes(intent.marker)), 'Echo assistant response missing');
  process.stdout.write('Compose smoke passed: HTTPS, web/docs, authentication, Echo, SSE and retained conversation.\n');
}
if (require.main === module) smoke(path.resolve(process.argv[2] ?? __dirname)).catch(error => {process.stderr.write(error.message + '\n'); process.exitCode = 1;});

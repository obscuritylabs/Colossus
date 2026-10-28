'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const vm = require('node:vm');
const { EventEmitter } = require('node:events');
const sourcePath = path.join(__dirname, 'hosted-probe.cjs');
const probe = require('./hosted-probe.cjs');
const deploymentId = '1234567890abcdef1234567890abcdef';
const adminId = 'synthetic-admin';
const marker = 'Colossus local deployment ' + deploymentId;
const member = { project_id: 'local', user_id: adminId, subject: adminId, role: 'terminal_operator',
  permissions: ['read', 'execute', 'control', 'terminal'], revision: 1 };
const project = { id: 'local', name: 'Local Colossus', description: marker,
  archived: false, parent_project_id: null, revision: 1 };
const identity = { user: { id: adminId, active: true, is_admin: true }, memberships: [member] };
const error = code => Object.assign(new Error(code), { probeCode: code });
const evidence = { path: 'synthetic-private-response', bytes: 2, sha256: 'a'.repeat(64) };
const response = (status, value) => ({ status, value, evidence });
function context(stage, replies) {
  const calls = [];
  const saves = [];
  const state = { stage, id: 'local', ...(stage === 'planned' ? {} : { marker, adminUserId: adminId, projectRevision: 1 }) };
  const ctx = { manifest: { deploymentId, hosted: { project: state } }, adminId, calls, saves,
    save(value) { this.manifest.hosted.project = structuredClone(value); saves.push(structuredClone(value)); },
    intent(method, route, body) { return { method, route, body, sha256: 'f'.repeat(64) }; },
    async api(method, route, body) {
      calls.push({ method, route, body, stage: this.manifest.hosted.project.stage });
      const next = replies.shift();
      assert.ok(next, 'unexpected request');
      assert.equal(method, next[0]); assert.equal(route, next[1]);
      if (next[2] instanceof Error) throw next[2];
      return next[2];
    } };
  return ctx;
}

test('fresh setup persists both intents before its only two POSTs', async () => {
  const ctx = context('planned', [
    ['GET', '/api/projects/local', response(404, { error: 'not_found' })],
    ['POST', '/api/projects', response(200, { project })],
    ['GET', '/api/projects/local', response(200, { project })],
    ['GET', '/api/projects/local/members?limit=100', response(200, { members: [], next_cursor: null })],
    ['POST', '/api/projects/local/members', response(200, { member })],
    ['GET', '/api/projects/local/members?limit=100', response(200, { members: [member], next_cursor: null })],
  ]);
  await probe.initializeProject(ctx);
  assert.equal(ctx.manifest.hosted.project.stage, 'ready');
  const posts = ctx.calls.filter(row => row.method === 'POST');
  assert.deepEqual(posts.map(row => row.stage), ['create-intent', 'membership-intent']);
  assert.equal(posts[1].body.revision, 0);
  assert.equal(posts[1].body.role, 'terminal_operator');
});

test('lost project response reconciles GET without repeating POST', async () => {
  const ctx = context('planned', [
    ['GET', '/api/projects/local', response(404, {})],
    ['POST', '/api/projects', error('api-request-timeout')],
    ['GET', '/api/projects/local', response(404, {})],
  ]);
  await assert.rejects(probe.initializeProject(ctx), { probeCode: 'project-create-outcome-unresolved' });
  assert.equal(ctx.manifest.hosted.project.stage, 'create-intent');
  ctx.api = async method => { assert.equal(method, 'GET'); return response(404, {}); };
  await assert.rejects(probe.initializeProject(ctx), { probeCode: 'project-create-outcome-unresolved' });
});

test('lost membership response resumes only by observing the exact grant', async () => {
  const ctx = context('membership-intent', [
    ['GET', '/api/projects/local', response(200, { project })],
    ['GET', '/api/projects/local/members?limit=100', response(200, { members: [member], next_cursor: null })],
  ]);
  await probe.initializeProject(ctx);
  assert.equal(ctx.manifest.hosted.project.stage, 'ready');
  assert.ok(ctx.calls.every(row => row.method === 'GET'));
  const missing = context('membership-intent', [
    ['GET', '/api/projects/local', response(200, { project })],
    ['GET', '/api/projects/local/members?limit=100', response(200, { members: [], next_cursor: null })],
  ]);
  await assert.rejects(probe.initializeProject(missing), { probeCode: 'membership-create-outcome-unresolved' });
});

test('unknown same-ID project and changed initial project never receive grants', async () => {
  const existing = context('planned', [['GET', '/api/projects/local', response(200, { project })]]);
  await assert.rejects(probe.initializeProject(existing), { probeCode: 'unowned-project-already-exists' });
  const collision = context('create-intent', [['GET', '/api/projects/local', response(200, { project: { ...project, description: 'some other deployment' } })]]);
  await assert.rejects(probe.initializeProject(collision), { probeCode: 'project-ownership-mismatch' });
  const changed = context('project-ready', [
    ['GET', '/api/projects/local', response(200, { project: { ...project, revision: 2 } })],
    ['GET', '/api/projects/local/members?limit=100', response(200, { members: [], next_cursor: null })],
  ]);
  await assert.rejects(probe.initializeProject(changed), { probeCode: 'fresh-project-changed' });
});

test('ready initialize is a no-op and verification only reads metadata', async () => {
  const ctx = context('ready', []);
  await probe.initializeProject(ctx);
  assert.equal(ctx.calls.length, 0);
  ctx.api = async (method, route) => {
    assert.equal(method, 'GET');
    assert.ok(!route.endsWith('/models') && !route.endsWith('/connect'));
    ctx.calls.push({ method, route });
    if (route === '/api/me') return response(200, identity);
    if (route === '/api/projects/local') return response(200, { project: { ...project, name: 'User renamed this', revision: 12 } });
    if (route === '/api/projects/local/hosted') return response(200, { enabled: true, capabilities: { managed_files: true } });
    if (route === '/api/projects/local/chat') return response(200, { enabled: true });
    return response(200, {});
  };
  const result = await probe.verifyHosted(ctx);
  assert.equal(result.adminExplicitExecute, true);
  assert.equal(result.agentTurns, 0);
  assert.equal(ctx.saves.length, 0);
  assert.equal(ctx.calls.length, 16);
});

test('ready changed role fails without resetting any authority', async () => {
  const ctx = context('ready', [['GET', '/api/me', response(200, { ...identity,
    memberships: [{ ...member, role: 'viewer', permissions: ['read'] }] })]]);
  await assert.rejects(probe.verifyHosted(ctx), { probeCode: 'explicit-terminal-membership-required' });
  assert.equal(ctx.saves.length, 0);
  assert.ok(ctx.calls.every(row => row.method === 'GET'));
});

test('cookie host, secure flag, expiry and uniqueness are enforced without values in errors', () => {
  const line = '#HttpOnly_colossus.localhost\tFALSE\t/\tTRUE\t0\tcolossus_session\tsynthetic-cookie';
  assert.equal(probe.cookieHeader('# Netscape HTTP Cookie File\n' + line), 'colossus_session=synthetic-cookie');
  for (const bad of [line.replace('colossus.localhost', 'evil.test'), line.replace('TRUE', 'FALSE'),
    line.replace('\t0\t', '\t1\t'), line + '\n' + line]) {
    assert.throws(() => probe.cookieHeader(bad), error => !error.message.includes('synthetic-cookie'));
  }
});

test('production transport pins loopback, CA, hostname, deadlines and refuses redirects and oversized bodies', async t => {
  const temporaryParent = fs.realpathSync(os.tmpdir());
  const root = fs.mkdtempSync(path.join(temporaryParent, 'colossus-hosted-probe-'));
  t.after(() => {
    assert.equal(fs.lstatSync(root).isSymbolicLink(), false);
    const actual = fs.realpathSync(root);
    assert.equal(actual, root);
    assert.equal(path.dirname(actual), temporaryParent);
    assert.match(path.basename(actual), /^colossus-hosted-probe-/);
    fs.rmSync(actual, { recursive: true, force: true });
  });
  const privateRoot = path.join(root, 'fixtures', '.colossus-tests', 'helm-synthetic-' + deploymentId);
  const publicRoot = path.join(root, 'public');
  fs.mkdirSync(privateRoot, { recursive: true }); fs.mkdirSync(publicRoot);
  const json = (name, value) => fs.writeFileSync(name, JSON.stringify(value));
  json(path.join(publicRoot, 'manifest.json'), { schemaVersion: 3, profile: 'full-hosted-v1', purpose: 'ColossusLocalHelmHarness',
    deploymentId, sourceCommit: 'a'.repeat(40), rootPath: publicRoot, clusterName: 'colossus-synthetic', namespace: 'colossus-synthetic', port: 8443,
    hosted: { projectId: 'local', project: { id: 'local', stage: 'planned' }, privateAllocation: { path: privateRoot, id: deploymentId, allocated: true, removed: false } } });
  json(path.join(privateRoot, 'ownership.json'), { purpose: 'ColossusLocalHelmPrivate', deploymentId, rootPath: publicRoot, privatePath: privateRoot });
  json(path.join(privateRoot, 'identity.json'), identity);
  fs.writeFileSync(path.join(privateRoot, 'smoke-cookie.txt'), 'colossus.localhost\tFALSE\t/\tTRUE\t0\tcolossus_session\tsynthetic-cookie\n');
  fs.writeFileSync(path.join(privateRoot, 'browser-cert.pem'), '-----BEGIN CERTIFICATE-----\nsynthetic\n-----END CERTIFICATE-----\n');
  let reply = { status: 200, body: '{}' };
  let clock = Date.now();
  let timeoutCallback;
  const deadlines = [];
  const stubHttps = { request(options, callback) {
    assert.equal(options.hostname, 'colossus.localhost'); assert.equal(options.servername, 'colossus.localhost');
    assert.equal(options.rejectUnauthorized, true); assert.equal(options.headers.Origin, 'https://colossus.localhost:8443');
    assert.ok(Buffer.isBuffer(options.ca)); assert.equal(options.agent, false);
    options.lookup('colossus.localhost', { all: true }, (err, addresses) => {
      assert.equal(err, null); assert.equal(addresses[0].address, '127.0.0.1'); assert.equal(addresses[0].family, 4);
    });
    const request = new EventEmitter();
    request.destroy = () => {};
    request.end = () => setImmediate(() => {
      if (reply.hang) return;
      const response = new EventEmitter(); response.statusCode = reply.status; response.headers = { 'content-type': 'application/json' };
      callback(response); response.emit('data', Buffer.from(reply.body)); response.emit('end');
    });
    return request;
  } };
  const module = { exports: {} };
  vm.runInNewContext(fs.readFileSync(sourcePath, 'utf8') + '\nmodule.exports.context = createContext;', {
    require: name => name === 'node:https' ? stubHttps : require(name), module, process, Buffer, clearTimeout,
    Date: class extends Date { static now() { return clock; } },
    setTimeout(callback, milliseconds) {
      deadlines.push(milliseconds); timeoutCallback = callback;
      return setTimeout(callback, milliseconds);
    },
  }, { filename: sourcePath });
  const ctx = module.exports.context(publicRoot, 8443);
  assert.equal((await ctx.api('GET', '/api/me')).status, 200);
  await assert.rejects(ctx.api('POST', '/api/projects/local/chat/connect', {}), { probeCode: 'request-effect-not-allowed' });
  reply = { status: 302, body: '{}' };
  await assert.rejects(ctx.api('GET', '/api/me'), { probeCode: 'unexpected-api-status' });
  reply = { status: 200, body: 'x'.repeat(2 * 1024 * 1024 + 1) };
  await assert.rejects(ctx.api('GET', '/api/me'), { probeCode: 'oversized-api-response' });
  reply = { hang: true };
  const timedOut = ctx.api('GET', '/api/me');
  setImmediate(() => timeoutCallback());
  await assert.rejects(timedOut, { probeCode: 'api-request-timeout' });
  assert.ok(deadlines.every(milliseconds => milliseconds > 0 && milliseconds <= 30_000));
  clock += 300_001;
  await assert.rejects(ctx.api('GET', '/api/me'), { probeCode: 'total-probe-deadline' });
  // No actual network requests, credentials or cluster resources are used.
});

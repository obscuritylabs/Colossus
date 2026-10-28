#!/usr/bin/env node
'use strict';

// The PowerShell caller verifies the allocation's Windows owner/SYSTEM-only ACL
// before invocation. Node retains all response bodies inside that same allocation.
const fs = require('node:fs');
const path = require('node:path');
const https = require('node:https');
const crypto = require('node:crypto');

const HOST = 'colossus.localhost';
const PROJECT = 'local';
const ROLE = 'terminal_operator';
const PERMISSIONS = ['control', 'execute', 'read', 'terminal'];
const MAX_BYTES = 2 * 1024 * 1024;
const REQUEST_MS = 30_000;
const TOTAL_MS = 300_000;
const STAGES = ['planned', 'create-intent', 'project-ready', 'membership-intent', 'ready'];
const sha256 = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const fail = code => { throw Object.assign(new Error(code), { probeCode: code }); };
const requireThat = (ok, code) => { if (!ok) fail(code); };
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const identifier = value => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const samePath = (left, right) => path.resolve(left).toLowerCase() === path.resolve(right).toLowerCase();
const within = (parent, child) => {
  const relative = path.relative(parent, child);
  return relative === '' || (!path.isAbsolute(relative) && relative !== '..' && !relative.startsWith('..' + path.sep));
};

function directoryWithoutLinks(directory) {
  requireThat(path.isAbsolute(directory), 'absolute-path-required');
  let cursor = path.resolve(directory);
  for (;;) {
    const stat = fs.lstatSync(cursor);
    requireThat(stat.isDirectory() && !stat.isSymbolicLink(), 'linked-directory-refused');
    const parent = path.dirname(cursor);
    if (parent === cursor) break;
    cursor = parent;
  }
}

function readFile(filename, limit = MAX_BYTES) {
  directoryWithoutLinks(path.dirname(filename));
  const stat = fs.lstatSync(filename);
  requireThat(stat.isFile() && !stat.isSymbolicLink() && stat.nlink === 1 && stat.size <= limit,
    'unsafe-or-oversized-file');
  const bytes = fs.readFileSync(filename);
  requireThat(bytes.length <= limit, 'oversized-file');
  return bytes;
}

function readJson(filename) {
  try { return JSON.parse(readFile(filename).toString('utf8').replace(/^\uFEFF/, '')); }
  catch (error) { if (error.probeCode) throw error; fail('invalid-private-or-manifest-json'); }
}

function writeNew(filename, bytes) {
  directoryWithoutLinks(path.dirname(filename));
  const fd = fs.openSync(filename, 'wx', 0o600);
  try { fs.writeFileSync(fd, bytes); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
}

function replaceJson(filename, value, expectedHash) {
  directoryWithoutLinks(path.dirname(filename));
  if (fs.existsSync(filename)) {
    const previous = readFile(filename);
    if (expectedHash !== undefined) requireThat(sha256(previous) === expectedHash, 'manifest-changed-concurrently');
  } else requireThat(expectedHash === undefined, 'manifest-disappeared');
  const bytes = Buffer.from(JSON.stringify(value, null, 2) + '\n');
  requireThat(bytes.length <= MAX_BYTES, 'oversized-local-receipt');
  const temporary = filename + '.' + crypto.randomUUID() + '.tmp';
  writeNew(temporary, bytes);
  fs.renameSync(temporary, filename);
  return sha256(bytes);
}

function cookieHeader(jar, now = Date.now()) {
  const selected = [];
  for (let line of jar.split(/\r?\n/)) {
    if (line.startsWith('#HttpOnly_')) line = line.slice('#HttpOnly_'.length);
    else if (line.startsWith('#') || !line.trim()) continue;
    const fields = line.split('\t');
    requireThat(fields.length === 7, 'invalid-cookie-jar');
    if (fields[5] !== 'colossus_session') continue;
    requireThat(fields[0] === HOST && fields[1] === 'FALSE' && fields[2] === '/' && fields[3] === 'TRUE',
      'cookie-origin-mismatch');
    requireThat(/^\d+$/.test(fields[4]) && (fields[4] === '0' || Number(fields[4]) * 1000 > now), 'expired-cookie');
    requireThat(/^[\x21-\x7e]+$/.test(fields[6]) && !/[;,"\\]/.test(fields[6]), 'invalid-session-cookie');
    selected.push(fields[6]);
  }
  requireThat(selected.length === 1, 'session-cookie-missing-or-ambiguous');
  return 'colossus_session=' + selected[0];
}

function checkIdentity(value, expectedId) {
  const user = value?.user;
  requireThat(object(user) && identifier(user.id) && user.active === true && user.is_admin === true,
    'active-bootstrap-admin-required');
  if (expectedId !== undefined) requireThat(user.id === expectedId, 'bootstrap-identity-changed');
  requireThat(Array.isArray(value.memberships), 'invalid-identity-memberships');
  return user.id;
}

function checkProject(value, marker, fresh = false) {
  const project = value?.project;
  requireThat(object(project) && project.id === PROJECT && project.description === marker && project.archived === false &&
    project.parent_project_id === null && Number.isSafeInteger(project.revision) && project.revision > 0,
  'project-ownership-mismatch');
  if (fresh) requireThat(project.name === 'Local Colossus' && project.revision === 1, 'fresh-project-changed');
  return project;
}

function checkMember(member, userId) {
  requireThat(object(member) && member.project_id === PROJECT && member.user_id === userId && member.subject === userId &&
    member.role === ROLE && Array.isArray(member.permissions) &&
    JSON.stringify([...member.permissions].sort()) === JSON.stringify(PERMISSIONS) &&
    Number.isSafeInteger(member.revision) && member.revision > 0, 'explicit-terminal-membership-required');
  return member;
}

function membersPage(value) {
  requireThat(Array.isArray(value?.members) && value.next_cursor === null, 'initial-membership-page-incomplete');
  return value.members;
}

// Every mutation has a durable intent. An uncertain or interrupted POST is only
// reconciled with GET; even a subsequent 404 never authorizes another POST.
async function initializeProject(ctx) {
  let state = ctx.manifest.hosted.project;
  if (state.stage === 'ready') return;
  const marker = 'Colossus local deployment ' + ctx.manifest.deploymentId;
  requireThat(state.marker === undefined || state.marker === marker, 'project-marker-changed');
  requireThat(state.adminUserId === undefined || state.adminUserId === ctx.adminId, 'bootstrap-identity-changed');
  if (state.stage !== 'planned') requireThat(state.marker === marker && state.adminUserId === ctx.adminId, 'incomplete-project-intent');
  let current = await ctx.api('GET', '/api/projects/local', undefined, [200, 404]);
  if (state.stage === 'planned') {
    requireThat(current.status === 404, 'unowned-project-already-exists');
    const body = { id: PROJECT, name: 'Local Colossus', description: marker, parent_project_id: null };
    ctx.save({ ...state, marker, adminUserId: ctx.adminId, stage: 'create-intent',
      createIntent: ctx.intent('POST', '/api/projects', body) });
    state = ctx.manifest.hosted.project;
    try {
      const response = await ctx.api('POST', '/api/projects', body, [200]);
      checkProject(response.value, marker, true);
      ctx.save({ ...state, createResponse: response.evidence });
    } catch (error) {
      // Preserve the intent and raw response even when its delivery was uncertain.
      if (!error.probeCode) throw error;
    }
    current = await ctx.api('GET', '/api/projects/local', undefined, [200, 404]);
  }
  requireThat(current.status === 200, 'project-create-outcome-unresolved');
  const project = checkProject(current.value, marker, true);
  if (state.stage === 'create-intent') {
    ctx.save({ ...ctx.manifest.hosted.project, stage: 'project-ready', projectRevision: project.revision,
      projectEvidence: current.evidence });
  }
  state = ctx.manifest.hosted.project;
  const page = await ctx.api('GET', '/api/projects/local/members?limit=100');
  const members = membersPage(page.value);
  if (state.stage === 'membership-intent') {
    requireThat(members.length === 1, 'membership-create-outcome-unresolved');
    const member = checkMember(members[0], ctx.adminId);
    ctx.save({ ...state, stage: 'ready', membershipRevision: member.revision, membershipEvidence: page.evidence });
    return;
  }
  requireThat(state.stage === 'project-ready' && members.length === 0, 'fresh-project-already-in-use');
  // The create revision is retained. Changed project metadata is never overwritten.
  requireThat(project.revision === state.projectRevision, 'fresh-project-revision-changed');
  const body = { user_id: ctx.adminId, role: ROLE, revision: 0 };
  ctx.save({ ...state, stage: 'membership-intent', membershipIntent: ctx.intent('POST', '/api/projects/local/members', body) });
  state = ctx.manifest.hosted.project;
  try {
    const response = await ctx.api('POST', '/api/projects/local/members', body, [200]);
    checkMember(response.value?.member, ctx.adminId);
    ctx.save({ ...state, membershipResponse: response.evidence });
  } catch (error) {
    if (!error.probeCode) throw error;
  }
  const reconciled = await ctx.api('GET', '/api/projects/local/members?limit=100');
  const rows = membersPage(reconciled.value);
  requireThat(rows.length === 1, 'membership-create-outcome-unresolved');
  const member = checkMember(rows[0], ctx.adminId);
  ctx.save({ ...ctx.manifest.hosted.project, stage: 'ready', membershipRevision: member.revision,
    membershipEvidence: reconciled.evidence });
}

async function verifyHosted(ctx) {
  const state = ctx.manifest.hosted.project;
  requireThat(state.stage === 'ready' && state.adminUserId === ctx.adminId &&
    state.marker === 'Colossus local deployment ' + ctx.manifest.deploymentId, 'hosted-initialization-required');
  const identity = await ctx.api('GET', '/api/me');
  checkIdentity(identity.value, ctx.adminId);
  const membership = identity.value.memberships.filter(row => row.project_id === PROJECT && row.user_id === ctx.adminId);
  requireThat(membership.length === 1, 'explicit-membership-not-found');
  checkMember(membership[0], ctx.adminId);
  checkProject((await ctx.api('GET', '/api/projects/local')).value, state.marker);
  // Later administration may add other members. Verify the retained grant only;
  // do not reset roles, project metadata, providers, credentials or personal data.
  for (const route of [
    '/api/projects/local/members?limit=100',
    '/api/admin/hosted-settings',
    '/api/admin/hosted-settings/provider-presets',
    '/api/admin/credential-store',
    '/api/projects/local/chat/settings',
    '/api/projects/local/chat/settings/provider-presets',
    '/api/projects/local/chat/credential-store/project',
    '/api/projects/local/chat/credential-store/personal',
    '/api/projects/local/chat/accounts',
    '/api/projects/local/chat/variables/personal',
    '/api/projects/local/chat/providers',
    '/api/projects/local/chat/connections',
  ]) await ctx.api('GET', route);
  const hosted = (await ctx.api('GET', '/api/projects/local/hosted')).value;
  const chat = (await ctx.api('GET', '/api/projects/local/chat')).value;
  requireThat(hosted?.enabled === true && hosted.capabilities?.managed_files === true && chat?.enabled === true,
    'hosted-discovery-disabled');
  return { projectId: PROJECT, adminUserId: ctx.adminId, role: ROLE, permissions: [...PERMISSIONS],
    adminExplicitExecute: true, hostedEnabled: true, zeroAgentTurns: true, agentTurns: 0 };
}

function createContext(root, port) {
  directoryWithoutLinks(root);
  const manifestPath = path.join(root, 'manifest.json');
  const bytes = readFile(manifestPath);
  let manifestHash = sha256(bytes);
  const manifest = readJson(manifestPath);
  requireThat(manifest.schemaVersion === 3 && manifest.profile === 'full-hosted-v1' &&
    manifest.purpose === 'ColossusLocalHelmHarness' && samePath(manifest.rootPath, root) &&
    /^[a-f0-9]{32}$/.test(manifest.deploymentId) && manifest.port === port &&
    /^[a-f0-9]{40}$/.test(manifest.sourceCommit) &&
    /^colossus-[a-z][a-z0-9-]{0,23}$/.test(manifest.clusterName) && manifest.namespace === manifest.clusterName &&
    manifest.hosted?.projectId === PROJECT && manifest.hosted.project?.id === PROJECT &&
    STAGES.includes(manifest.hosted.project.stage), 'invalid-full-hosted-manifest');
  const allocation = manifest.hosted.privateAllocation;
  requireThat(object(allocation) && path.isAbsolute(allocation.path) && allocation.id === manifest.deploymentId &&
    allocation.allocated === true && allocation.removed === false, 'private-allocation-required');
  const privateRoot = path.resolve(allocation.path);
  directoryWithoutLinks(privateRoot);
  requireThat(path.basename(privateRoot) === `helm-${manifest.clusterName.slice('colossus-'.length)}-${manifest.deploymentId}` &&
    path.basename(path.dirname(privateRoot)) === '.colossus-tests' &&
    !within(root, privateRoot) && !within(privateRoot, root),
  'private-allocation-boundary-mismatch');
  const owner = readJson(path.join(privateRoot, 'ownership.json'));
  requireThat(owner.purpose === 'ColossusLocalHelmPrivate' && owner.deploymentId === manifest.deploymentId &&
    samePath(owner.rootPath, root) && samePath(owner.privatePath, privateRoot), 'private-allocation-ownership-mismatch');
  const cookie = cookieHeader(readFile(path.join(privateRoot, 'smoke-cookie.txt'), 64 * 1024).toString('utf8'));
  const identity = readJson(path.join(privateRoot, 'identity.json'));
  const adminId = checkIdentity(identity);
  const ca = readFile(path.join(privateRoot, 'browser-cert.pem'));
  requireThat(ca.includes(Buffer.from('-----BEGIN CERTIFICATE-----')), 'browser-ca-required');
  const runId = crypto.randomUUID();
  const runRoot = path.join(privateRoot, 'hosted-probe-' + runId);
  fs.mkdirSync(runRoot, { mode: 0o700 });
  const origin = `https://${HOST}${port === 443 ? '' : ':' + port}`;
  const started = Date.now();
  const requests = [];
  const ctx = { manifest, adminId, origin, runId, requests, privateRoot, manifestPath, runRoot,
    save(state) {
      ctx.manifest.hosted.project = state;
      manifestHash = replaceJson(manifestPath, ctx.manifest, manifestHash);
    },
    intent(method, route, body) {
      const value = { schemaVersion: 1, deploymentId: manifest.deploymentId, adminUserId: adminId,
        method, route, body, atUtc: new Date().toISOString() };
      const filename = path.join(runRoot, `intent-${crypto.randomUUID()}.json`);
      const bodyBytes = Buffer.from(JSON.stringify(value, null, 2) + '\n');
      writeNew(filename, bodyBytes);
      return { atUtc: value.atUtc, path: filename, bytes: bodyBytes.length, sha256: sha256(bodyBytes) };
    },
    async api(method, route, body, statuses = [200]) {
      requireThat(method === 'GET' || (method === 'POST' &&
        ['/api/projects', '/api/projects/local/members'].includes(route)), 'request-effect-not-allowed');
      requireThat(/^\/api\/[A-Za-z0-9/?=&_-]+$/.test(route), 'invalid-api-route');
      const remaining = TOTAL_MS - (Date.now() - started);
      requireThat(remaining > 0, 'total-probe-deadline');
      const payload = body === undefined ? undefined : Buffer.from(JSON.stringify(body));
      requireThat(!payload || payload.length <= 64 * 1024, 'oversized-request');
      const sequence = requests.length + 1;
      const summary = { sequence, method, route, status: null, bytes: 0, sha256: null };
      requests.push(summary);
      const result = await new Promise((resolve, reject) => {
        let settled = false;
        let timer;
        const finish = (error, value) => {
          if (settled) return;
          settled = true;
          clearTimeout(timer);
          if (error) reject(error); else resolve(value);
        };
        const request = https.request({ protocol: 'https:', hostname: HOST, port, path: route, method,
          ca, rejectUnauthorized: true, servername: HOST, agent: false,
          lookup(_hostname, options, callback) {
            callback(null, options?.all ? [{ address: '127.0.0.1', family: 4 }] : '127.0.0.1', 4);
          },
          headers: { Cookie: cookie, Origin: origin, 'X-Colossus-CSRF': '1', Accept: 'application/json',
            ...(payload ? { 'Content-Type': 'application/json', 'Content-Length': payload.length } : {}) },
        }, response => {
          const chunks = [];
          let size = 0;
          response.on('data', chunk => {
            size += chunk.length;
            if (size > MAX_BYTES) { request.destroy(); finish(Object.assign(new Error(), { probeCode: 'oversized-api-response' })); }
            else chunks.push(chunk);
          });
          response.on('error', () => finish(Object.assign(new Error(), { probeCode: 'api-response-incomplete' })));
          response.on('end', () => {
            if (!settled) finish(null, { status: response.statusCode, headers: response.headers, bytes: Buffer.concat(chunks) });
          });
        });
        timer = setTimeout(() => {
          request.destroy(); finish(Object.assign(new Error(), { probeCode: 'api-request-timeout' }));
        }, Math.min(REQUEST_MS, remaining));
        request.on('error', () => finish(Object.assign(new Error(), { probeCode: 'api-transport-failed' })));
        request.end(payload);
      });
      const filename = path.join(runRoot, `response-${String(sequence).padStart(3, '0')}.body`);
      writeNew(filename, result.bytes);
      writeNew(filename + '.json', Buffer.from(JSON.stringify({ method, route, status: result.status,
        headers: result.headers }, null, 2) + '\n'));
      Object.assign(summary, { status: result.status, bytes: result.bytes.length, sha256: sha256(result.bytes) });
      const evidence = { path: filename, bytes: result.bytes.length, sha256: summary.sha256 };
      requireThat(statuses.includes(result.status), 'unexpected-api-status');
      requireThat(result.status < 300 || result.status >= 400, 'api-redirect-refused');
      let value;
      try { value = JSON.parse(result.bytes.toString('utf8')); }
      catch { fail('invalid-api-json'); }
      requireThat(object(value), 'invalid-api-object');
      return { status: result.status, value, evidence };
    },
  };
  return ctx;
}

async function main(argv = process.argv.slice(2)) {
  requireThat(argv.length === 3 && path.isAbsolute(argv[0]) && /^\d{1,5}$/.test(argv[1]) &&
    Number(argv[1]) >= 1 && Number(argv[1]) <= 65535 && ['initialize', 'verify'].includes(argv[2]), 'invalid-probe-arguments');
  requireThat(Number(process.versions.node.split('.')[0]) >= 22, 'node-22-required');
  const root = path.resolve(argv[0]);
  directoryWithoutLinks(root);
  const lock = path.join(root, '.hosted-probe.lock');
  // A stale lock after process loss needs explicit inspection, never automatic reuse.
  writeNew(lock, Buffer.from(JSON.stringify({ pid: process.pid, nonce: crypto.randomUUID() }) + '\n'));
  let ctx;
  try {
    ctx = createContext(root, Number(argv[1]));
    checkIdentity((await ctx.api('GET', '/api/me')).value, ctx.adminId);
    if (argv[2] === 'initialize') await initializeProject(ctx);
    const verified = await verifyHosted(ctx);
    const receipt = { schemaVersion: 1, purpose: 'ColossusLocalHostedVerification', passed: true,
      profile: ctx.manifest.profile, deploymentId: ctx.manifest.deploymentId, sourceCommit: ctx.manifest.sourceCommit,
      origin: ctx.origin, mode: argv[2], ...verified, projectStage: ctx.manifest.hosted.project.stage,
      requests: ctx.requests, privateEvidencePath: ctx.runRoot, verifiedAtUtc: new Date().toISOString() };
    replaceJson(path.join(root, 'hosted-verification.json'), receipt);
    process.stdout.write('Hosted metadata verified; local project has explicit Execute and Terminal permissions. Agent turns: 0.\n');
    return receipt;
  } catch (error) {
    if (ctx) replaceJson(path.join(root, 'hosted-verification.json'), {
      schemaVersion: 1, purpose: 'ColossusLocalHostedVerification', passed: false,
      deploymentId: ctx.manifest.deploymentId, profile: ctx.manifest.profile, origin: ctx.origin,
      projectId: PROJECT, projectStage: ctx.manifest.hosted.project.stage,
      errorCode: error.probeCode || 'local-probe-failed', zeroAgentTurns: true, agentTurns: 0,
      requests: ctx.requests, privateEvidencePath: ctx.runRoot, verifiedAtUtc: new Date().toISOString(),
    });
    throw error;
  } finally { fs.unlinkSync(lock); }
}

module.exports = { cookieHeader, checkIdentity, checkProject, checkMember, initializeProject, verifyHosted, main };
if (require.main === module) main().catch(error => {
  process.stderr.write(`Hosted probe failed (${error.probeCode || 'local-probe-failed'}). State and private evidence retained; no automatic retry.\n`);
  process.exitCode = 1;
});

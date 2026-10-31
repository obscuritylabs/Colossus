// One-shot setup container. Later restarts preserve administrator edits.
const fs = require('node:fs');
const https = require('node:https');
const assert = require('node:assert/strict');
const root = '/run/colossus/';
async function bootstrap() {
  if (fs.existsSync(root + 'complete')) return;
  let cookie;
  const api = (method, route, body, statuses = [200]) => new Promise((resolve, reject) => {
    const payload = body === undefined ? undefined : Buffer.from(JSON.stringify(body));
    const request = https.request('https://browser:8443' + route, {
      method, ca: fs.readFileSync(root + 'ca.pem'), rejectUnauthorized: true,
      headers: {Host: new URL(process.env.COLOSSUS_PUBLIC_ORIGIN).host, Origin: process.env.COLOSSUS_PUBLIC_ORIGIN, 'X-Colossus-CSRF': '1',
        ...(cookie ? {Cookie: cookie} : {}), ...(payload ? {'Content-Type': 'application/json', 'Content-Length': payload.length} : {})},
    }, response => {
      if (!statuses.includes(response.statusCode)) {response.resume(); reject(new Error(`Bootstrap ${method} ${route}: HTTP ${response.statusCode}`)); return;}
      if (route === '/auth/local') {
        cookie = (response.headers['set-cookie'] ?? []).find(value => value.startsWith('colossus_session='))?.split(';')[0];
        assert(cookie, 'session cookie missing');
      }
      let text = '';
      response.setEncoding('utf8');
      response.on('data', chunk => {text += chunk; if (Buffer.byteLength(text) > 1024 * 1024) request.destroy(new Error('bootstrap response bound exceeded'));});
      response.on('error', reject);
      response.on('end', () => {try {resolve(text ? JSON.parse(text) : {});} catch (error) {reject(error);}});
    });
    request.setTimeout(30000, () => request.destroy(new Error('bootstrap timeout')));
    request.on('error', reject); request.end(payload);
  });
  await api('POST', '/auth/local', {username: 'admin', password: fs.readFileSync(root + 'admin-password', 'utf8')}, [200, 204]);
  const user = (await api('GET', '/api/me')).user;
  assert(user.is_admin);
  const marker = 'Colossus Compose default project';
  let project = (await api('GET', '/api/projects/local', undefined, [200, 404])).project;
  if (!project) project = (await api('POST', '/api/projects', {id: 'local', name: 'Local Colossus', description: marker, parent_project_id: null})).project;
  assert.equal(project.description, marker, 'local project is not owned by setup');
  const members = (await api('GET', '/api/projects/local/members?limit=100')).members;
  const existing = members.find(member => member.user_id === user.id);
  if (!existing) await api('POST', '/api/projects/local/members', {user_id: user.id, role: 'terminal_operator', revision: 0});
  else assert.equal(existing.role, 'terminal_operator', 'setup will not overwrite a changed membership');
  fs.writeFileSync(root + 'complete', 'Local Colossus initialized\n', {flag: 'wx', mode: 0o600});
  process.stdout.write('Local Colossus Chat is ready.\n');
}
bootstrap().catch(error => {process.stderr.write(error.message + '\n'); process.exitCode = 1;});

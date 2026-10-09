import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";

const image = process.argv[2];
if (
  process.argv.length !== 3 ||
  !/^[a-zA-Z0-9][a-zA-Z0-9._:/@-]{0,511}$/.test(image ?? "")
) {
  throw new Error("usage: node deploy/documentation/smoke.mjs IMAGE");
}
const name = `colossus-documentation-smoke-${randomUUID().slice(0, 12)}`;
const origin = "http://documentation.test";
const base = `${origin}/docs/`;

function docker(args, input) {
  const result = spawnSync("docker", args, {
    input,
    maxBuffer: 16 * 1024 * 1024,
    timeout: 30_000,
  });
  if (result.error || result.status !== 0) {
    throw new Error(
      `Docker ${args[0]} failed: ${result.error?.message ?? result.stderr?.toString()}`,
    );
  }
  return result.stdout;
}

function request(path, method = "GET") {
  assert.match(path, /^\/[\x21-\x7e]*$/);
  const raw = docker(
    ["exec", "-i", name, "/bin/busybox", "nc", "-w", "2", "127.0.0.1", "8080"],
    `${method} ${path} HTTP/1.0\r\nHost: documentation.test\r\nConnection: close\r\nContent-Length: 0\r\n\r\n`,
  );
  const end = raw.indexOf("\r\n\r\n");
  assert.ok(end > 0, "static server must return an HTTP response");
  const lines = raw.subarray(0, end).toString().split("\r\n");
  const status = Number(lines.shift().split(" ")[1]);
  const headers = Object.fromEntries(
    lines.map((line) => {
      const at = line.indexOf(":");
      return [line.slice(0, at).toLowerCase(), line.slice(at + 1).trim()];
    }),
  );
  return { status, headers, body: raw.subarray(end + 4) };
}

function successful(path) {
  const response = request(path);
  assert.equal(response.status, 200, path);
  assert.ok(response.body.length > 0, path);
  assert.equal(response.headers["x-content-type-options"], "nosniff");
  assert.equal(response.headers["x-frame-options"], "DENY");
  assert.match(
    response.headers["content-security-policy"],
    /connect-src 'self'/,
  );
  assert.equal(response.headers["set-cookie"], undefined);
  return response;
}

const verified = [];
let started = false;
try {
  docker([
    "run",
    "--detach",
    "--name",
    name,
    "--network",
    "none",
    "--read-only",
    "--user",
    "10001:10001",
    "--cap-drop",
    "ALL",
    "--security-opt",
    "no-new-privileges",
    "--memory",
    "128m",
    "--cpus",
    "0.25",
    "--pids-limit",
    "32",
    "--tmpfs",
    "/tmp:rw,noexec,nosuid,size=16m,mode=1777",
    image,
  ]);
  started = true;
  for (let attempt = 0; ; attempt++) {
    try {
      successful("/health/live");
      break;
    } catch (error) {
      if (attempt === 19) throw error;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
  const [container] = JSON.parse(docker(["inspect", name]).toString());
  assert.equal(container.Config.User, "10001:10001");
  assert.equal(container.HostConfig.ReadonlyRootfs, true);
  assert.equal(container.HostConfig.NetworkMode, "none");
  assert.deepEqual(container.HostConfig.CapDrop, ["ALL"]);
  assert.ok(container.HostConfig.SecurityOpt.includes("no-new-privileges"));
  verified.push("non-root/read-only/no-network/capability boundary");

  for (const path of ["/", "/docs"]) {
    const response = request(path);
    assert.equal(response.status, 308);
    assert.equal(response.headers.location, "/docs/");
  }
  verified.push("relative mount redirects");

  for (const path of [
    "/docs/",
    "/docs/admin/cloud-control-plane/",
    "/docs/admin/documentation-hosting/",
    "/docs/develop/application-sdk/",
  ]) {
    const response = successful(path);
    assert.match(response.headers["content-type"], /^text\/html/);
    const html = response.body.toString();
    assert.match(html, /<meta name="generator" content="zensical-0\.0\.50">/);
    assert.doesNotMatch(html, /https:\/\/obscuritylabs\.github\.io\/Colossus/);
    const config = /<script[^>]*id="__config"[^>]*>([\s\S]*?)<\/script>/.exec(
      html,
    );
    assert.ok(config, "generated page configuration");
    const features = JSON.parse(config[1]).features;
    assert.ok(Array.isArray(features));
    assert.ok(!features.includes("navigation.instant"));
    assert.ok(!features.includes("navigation.instant.progress"));
    const canonical = /<link rel="canonical" href="([^"]+)"/.exec(html);
    assert.ok(
      canonical && new URL(canonical[1], base).pathname.startsWith("/docs/"),
      "portable canonical URL",
    );
  }
  verified.push("homepage/direct pages/local canonical URLs");

  const home = successful("/docs/");
  const assets = [...home.body.toString().matchAll(/(?:src|href)="([^"]+)"/g)]
    .map((match) => new URL(match[1], base))
    .filter(
      (url) => url.origin === origin && /\.(?:css|js|svg)$/.test(url.pathname),
    );
  assert.ok(assets.length >= 3);
  for (const url of assets) {
    assert.ok(url.pathname.startsWith("/docs/"));
    successful(url.pathname);
  }
  successful("/docs/assets/vendor/mermaid-11.15.0.min.js");
  verified.push("referenced local assets and pinned diagrams");

  const searchResponse = successful("/docs/search.json");
  assert.match(searchResponse.headers["content-type"], /application\/json/);
  const search = JSON.parse(searchResponse.body.toString());
  assert.ok(Array.isArray(search.items) && search.items.length > 100);
  const locations = search.items.map((item) => {
    assert.equal(typeof item.location, "string");
    assert.equal(typeof item.title, "string");
    assert.equal(typeof item.text, "string");
    const url = new URL(item.location, base);
    assert.equal(url.origin, origin);
    assert.ok(url.pathname.startsWith("/docs/"));
    return url.pathname;
  });
  for (const path of [
    "/docs/admin/cloud-control-plane/",
    "/docs/admin/documentation-hosting/",
    "/docs/develop/application-sdk/",
  ]) {
    assert.ok(locations.includes(path), `search includes ${path}`);
    successful(path);
  }
  verified.push("browser/agent search index and direct result URLs");

  const legacy = successful("/docs/GETTING_STARTED.html").body.toString();
  assert.match(legacy, /\/docs\/get-started\/quickstart\//);
  assert.doesNotMatch(legacy, /obscuritylabs\.github\.io|\/Colossus\//);
  successful("/docs/get-started/quickstart/");
  verified.push("portable legacy compatibility redirect");

  const head = request("/docs/", "HEAD");
  assert.equal(head.status, 200);
  assert.equal(head.body.length, 0);
  assert.ok(Number(head.headers["content-length"]) > 0);
  for (const path of [
    "/docs/missing-documentation-page/",
    "/docs/assets/missing.js",
    "/api/me",
    "/auth/login",
    "/etc/nginx/nginx.conf",
  ]) {
    assert.equal(request(path).status, 404, path);
  }
  assert.ok([400, 404].includes(request("/docs/%2e%2e/etc/passwd").status));
  assert.equal(request("/docs/", "POST").status, 405);
  verified.push("HEAD/missing pages/assets/reserved paths/traversal/non-GET");

  console.log(
    JSON.stringify(
      {
        image,
        image_id: container.Image,
        labels: container.Config.Labels,
        search_items: search.items.length,
        local_assets: assets.length,
        verified,
      },
      null,
      2,
    ),
  );
} finally {
  if (started) docker(["rm", "--force", name]);
}

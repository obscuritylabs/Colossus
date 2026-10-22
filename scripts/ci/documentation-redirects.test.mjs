import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";

const generator = fileURLToPath(new URL("../generate-doc-redirects", import.meta.url));

function fixture(t, manifest, mount) {
  const directory = mkdtempSync(join(tmpdir(), "colossus-doc-redirects-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const site = join(directory, "site");
  mkdirSync(site);
  const routes = join(directory, "routes.tsv");
  writeFileSync(routes, manifest);
  const args = [generator, routes, site];
  if (mount) args.push(mount);
  return { site, result: spawnSync("bash", args, { encoding: "utf8" }) };
}

function destination(html, hash) {
  let result;
  const script = /<script>([\s\S]*?)<\/script>/.exec(html)?.[1];
  assert.ok(script);
  runInNewContext(script, { window: { location: { hash, replace(value) { result = value; } } } });
  return result;
}

test("moved Cloud links preserve page-specific destinations on both publication roots", (t) => {
  for (const mount of [undefined, "/docs", "/"]) {
    const { site, result } = fixture(t,
      "/admin/cloud-control-plane/index.html\t/cloud/\tdeploy-on-kubernetes=/cloud/deployment/kubernetes/#install-the-release\n", mount);
    assert.equal(result.status, 0, result.stderr);
    const html = readFileSync(join(site, "admin/cloud-control-plane/index.html"), "utf8");
    const root = mount === "/" ? "" : mount ?? "/Colossus";
    assert.equal(destination(html, "#deploy-on-kubernetes"), `${root}/cloud/deployment/kubernetes/#install-the-release`);
    assert.equal(destination(html, ""), `${root}/cloud/`);
    assert.equal(destination(html, "#unknown"), `${root}/cloud/#unknown`);
    assert.equal(destination(html, "#//evil.example/"), `${root}/cloud/#//evil.example/`);
    if (mount) assert.doesNotMatch(html, /obscuritylabs\.github\.io|\/Colossus\//);
  }
});

test("existing HTML redirects retain renamed, dropped, and unknown anchors", (t) => {
  const { site, result } = fixture(t,
    "/SEARCH.html\t/use/web-search/\tconfiguration=prerequisites,removed=\n");
  assert.equal(result.status, 0, result.stderr);
  const html = readFileSync(join(site, "SEARCH.html"), "utf8");
  assert.equal(destination(html, "#configuration"), "/Colossus/use/web-search/#prerequisites");
  assert.equal(destination(html, "#removed"), "/Colossus/use/web-search/");
  assert.equal(destination(html, "#other"), "/Colossus/use/web-search/#other");
});

test("route generation rejects traversal and external or executable destinations", (t) => {
  for (const manifest of [
    "/admin/../index.html\t/cloud/\n",
    "/admin/cloud/index.html\thttps://evil.example/\n",
    "/admin/cloud/index.html\t/cloud/\told=//evil.example/\n",
    "/admin/cloud/index.html\t/cloud/\told=/cloud/../admin/\n",
    "/admin/cloud/index.html\t/cloud/\told=javascript:alert(1)\n",
  ]) {
    const { result } = fixture(t, manifest);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /invalid legacy path|target is not a clean directory URL|invalid fragment mapping/);
  }
});

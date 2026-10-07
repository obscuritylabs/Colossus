import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { validateViewContributions } from "../scripts/validate-manifest.mjs";

const manifest = JSON.parse(
  await readFile(new URL("../package.json", import.meta.url), "utf8"),
);
test("referenced webview assets and shared attribution remain in the VSIX allowlist", async () => {
  const [extension, build, ignore] = await Promise.all([
    readFile(new URL("../src/extension.ts", import.meta.url), "utf8"),
    readFile(new URL("../scripts/build.mjs", import.meta.url), "utf8"),
    readFile(new URL("../.vscodeignore", import.meta.url), "utf8"),
  ]);
  const included = new Set(ignore.split(/\r?\n/u));
  const referenced = [...extension.matchAll(/\basset\("([^"\n]+)"\)/gu)].map(
    (match) => match[1],
  );
  assert.ok(referenced.includes("shadcn.css"));
  for (const asset of [
    ...referenced,
    "shadcn-LICENSE.txt",
    "THIRD_PARTY_NOTICES.txt",
  ]) {
    assert.ok(
      included.has(`!dist/${asset}`),
      `Referenced or attributed asset would be omitted: ${asset}`,
    );
    assert.ok(
      build.includes(`dist/${asset}`),
      `Asset is not emitted by the build: ${asset}`,
    );
  }
});

test("all contributed views have valid VS Code containers and bundled icons", async () => {
  assert.doesNotThrow(() => validateViewContributions(manifest));
  for (const entries of Object.values(manifest.contributes.viewsContainers))
    for (const container of entries) {
      const icon = await readFile(
        new URL(`../${container.icon}`, import.meta.url),
        "utf8",
      );
      assert.match(icon, /<svg\b/);
    }
});

test("VS Code's container-ID restriction rejects the dotted-ID regression without forbidding dotted view IDs", () => {
  const invalid = structuredClone(manifest);
  invalid.contributes.viewsContainers.activitybar[0].id = "colossus.explorer";
  assert.throws(
    () => validateViewContributions(invalid),
    /Invalid view container ID: colossus\.explorer/,
  );
  assert.ok(
    manifest.contributes.views[
      manifest.contributes.viewsContainers.activitybar[0].id
    ].every((view) => view.id.includes(".")),
  );
  assert.doesNotThrow(() => validateViewContributions(manifest));
});

test("an orphaned view binding fails before VS Code can send it into Explorer", () => {
  const invalid = structuredClone(manifest);
  const id = invalid.contributes.viewsContainers.activitybar[0].id;
  invalid.contributes.views["missing-container"] =
    invalid.contributes.views[id];
  delete invalid.contributes.views[id];
  assert.throws(
    () => validateViewContributions(invalid),
    /undeclared container: missing-container/,
  );
});

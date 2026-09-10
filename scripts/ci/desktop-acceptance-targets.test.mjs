import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import {
  mkdtemp,
  mkdir,
  writeFile,
  copyFile,
  readFile,
  readdir,
  rm,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { acceptanceTargets } from "../../apps/desktop/scripts/acceptance-targets.mjs";

for (const variant of ["default", "relative", "absolute"]) {
  test(`Tauri external binaries cannot replace the acceptance runtime: ${variant}`, async () => {
    const root = await mkdtemp(join(tmpdir(), "colossus-acceptance-targets-"));
    try {
      const repository = join(root, "repository");
      const desktop = join(repository, "apps/desktop");
      const configured =
        variant === "default"
          ? undefined
          : variant === "relative"
            ? "shared"
            : join(root, "shared");
      const targets = acceptanceTargets(repository, desktop, configured);
      assert.equal(
        targets.runtime,
        resolve(repository, configured ?? "target"),
      );
      assert.notEqual(targets.runtime, targets.native);
      await mkdir(join(targets.runtime, "debug"), { recursive: true });
      await mkdir(join(targets.native, "debug"), { recursive: true });
      const staged = join(root, "staged-sidecar");
      await writeFile(staged, "old staged protocol");
      for (const name of ["colossus", "colossus-sidecar"]) {
        const current = join(targets.runtime, "debug", name);
        await writeFile(current, "current compiled protocol");
        // Reproduce tauri-build's externalBin publication after the runtime build.
        await copyFile(staged, join(targets.native, "debug", name));
        assert.equal(
          await readFile(current, "utf8"),
          "current compiled protocol",
        );
      }
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
}

test("approval acceptance preserves a failed plugin run's evidence", async () => {
  const repository = new URL("../../", import.meta.url);
  const outputs = [];
  for (const suite of ["plugin", "approval"]) {
    const script = await readFile(
      new URL(`apps/desktop/scripts/test-${suite}-runtime.mjs`, repository),
      "utf8",
    );
    const output = script.match(/"--output=(test-results\/[a-z-]+)"/u)?.[1];
    assert.ok(output, `${suite} runner must isolate its Playwright evidence`);
    outputs.push(output);
  }
  assert.notEqual(outputs[0], outputs[1]);
  assert.ok(
    !outputs.some((path, index) => path.startsWith(`${outputs[1 - index]}/`)),
  );

  const root = await mkdtemp(join(tmpdir(), "colossus-acceptance-evidence-"));
  try {
    const playwright = new URL(
      "apps/desktop/node_modules/@playwright/test/",
      repository,
    );
    await writeFile(
      join(root, "playwright.config.cjs"),
      'module.exports = { testDir: ".", testMatch: "fixture.spec.cjs", workers: 1, retries: 0 };',
    );
    await writeFile(
      join(root, "fixture.spec.cjs"),
      `const { test } = require(${JSON.stringify(fileURLToPath(playwright))});
test("evidence fixture", async ({}, info) => {
  if (process.env.COLOSSUS_EVIDENCE_FAILURE === "1") {
    const fs = require("node:fs/promises");
    await fs.mkdir(info.outputDir, { recursive: true });
    await fs.writeFile(info.outputPath("retained-trace.txt"), "plugin evidence");
    throw new Error("expected evidence fixture failure");
  }
});`,
    );
    const run = (index, failure) =>
      promisify(execFile)(
        process.execPath,
        [
          fileURLToPath(new URL("cli.js", playwright)),
          "test",
          "--reporter=line",
          `--output=${outputs[index]}`,
        ],
        {
          cwd: root,
          env: { ...process.env, COLOSSUS_EVIDENCE_FAILURE: failure },
          timeout: 15_000,
        },
      );
    await assert.rejects(run(0, "1"), { code: 1 });
    const retained = (
      await readdir(join(root, outputs[0]), { recursive: true })
    ).find((path) => path.endsWith("retained-trace.txt"));
    assert.ok(retained, "failed plugin run must retain its evidence");
    await run(1, "0");
    assert.equal(
      await readFile(join(root, outputs[0], retained), "utf8"),
      "plugin evidence",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

import assert from "node:assert/strict";
import { test } from "node:test";
import { resolve } from "node:path";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { spawnSync } from "node:child_process";
import { nativeExecutables } from "../scripts/cargo-native.mjs";

const packages = ["colossus-app-host", "colossus-sidecar"];
const artifact = (name, overrides = {}) => ({
  reason: "compiler-artifact",
  target: { name, kind: ["bin"] },
  profile: { test: false },
  executable: resolve("target/vscode", name),
  ...overrides,
});

test("native packaging uses the exact runnable Cargo artifacts", () => {
  const messages = [
    artifact("dependency"),
    ...packages.map((p) => artifact(p)),
  ];
  const paths = nativeExecutables(messages, packages);
  assert.deepEqual(paths, {
    "colossus-app-host": messages[1].executable,
    "colossus-sidecar": messages[2].executable,
  });
});

test("native packaging rejects test harnesses, missing and ambiguous artifacts", () => {
  const [host, sidecar] = packages.map((p) => artifact(p));
  assert.throws(
    () =>
      nativeExecutables(
        [{ ...host, profile: { test: true } }, sidecar],
        packages,
      ),
    /runnable native executable/u,
  );
  assert.throws(() => nativeExecutables([host], packages), /every native/u);
  assert.throws(
    () =>
      nativeExecutables(
        [host, { ...host, executable: resolve("old-host") }, sidecar],
        packages,
      ),
    /ambiguous/u,
  );
});

test("staging copies the reported Cargo files and rejects an incomplete artifact map", async () => {
  const fixture = await mkdtemp(resolve(tmpdir(), "colossus-native-stage-"));
  try {
    const paths = {};
    const bytes = Buffer.alloc(256);
    bytes.writeUInt16LE(0x5a4d);
    bytes.writeUInt32LE(128, 60);
    bytes.writeUInt32LE(0x4550, 128);
    bytes.writeUInt16LE(0x8664, 132);
    for (const name of packages) {
      paths[name] = resolve(fixture, `${name}-reported.exe`);
      await writeFile(paths[name], bytes);
    }
    const stage = (artifacts) =>
      spawnSync(
        process.execPath,
        [
          resolve("scripts/stage-native.mjs"),
          "win32-x64",
          "--cargo-artifacts",
          JSON.stringify(artifacts),
        ],
        { cwd: fixture, encoding: "utf8" },
      );
    const result = stage(paths);
    assert.equal(result.status, 0, result.stderr);
    const manifest = JSON.parse(
      await readFile(resolve(fixture, "native/manifest.json"), "utf8"),
    );
    const digest = createHash("sha256").update(bytes).digest("hex");
    for (const name of packages) {
      assert.equal(manifest[name], digest);
      assert.deepEqual(
        await readFile(resolve(fixture, `native/${name}.exe`)),
        bytes,
      );
    }
    assert.notEqual(stage(null).status, 0);
    assert.notEqual(stage({ [packages[0]]: paths[packages[0]] }).status, 0);
    // Failed input validation must also leave the last valid inventory untouched.
    assert.equal(
      JSON.parse(
        await readFile(resolve(fixture, "native/manifest.json"), "utf8"),
      )[packages[0]],
      digest,
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const patcher = join(repository, "scripts/patch-desktop-manifest-binding.mjs");
const prefix = "COLOSSUS_DESKTOP_RELEASE_MANIFEST_SHA256_V1=";
const suffix = ":END_COLOSSUS_DESKTOP_RELEASE_MANIFEST_SHA256";
const placeholder = `${prefix}${"0".repeat(64)}${suffix}`;

function manifest({
  targetTriple = "aarch64-apple-darwin",
  sidecar = "colossus-sidecar",
  cli = "colossus",
} = {}) {
  return `${JSON.stringify({
    schemaVersion: 2,
    targetTriple,
    profile: "release",
    releaseChannel: "developer_preview",
    sidecar: { fileName: sidecar, sha256: "a".repeat(64) },
    cli: { fileName: cli, sha256: "b".repeat(64) },
  })}\n`;
}

function executable(bindings) {
  const header = Buffer.alloc(32, 0);
  header.writeUInt32BE(0xfeedfacf, 0);
  return Buffer.concat([
    header,
    ...bindings.map((binding) => Buffer.from(binding, "ascii")),
    Buffer.alloc(32, 0),
  ]);
}

function portableExecutable(bindings) {
  const header = Buffer.alloc(160, 0);
  header.write("MZ", 0, "ascii");
  header.writeUInt32LE(128, 0x3c);
  header.write("PE\0\0", 128, "ascii");
  return Buffer.concat([
    header,
    ...bindings.map((binding) => Buffer.from(binding, "ascii")),
    Buffer.alloc(32, 0),
  ]);
}

function run(executablePath, manifestPath) {
  return spawnSync(
    process.execPath,
    [patcher, "--executable", executablePath, "--manifest", manifestPath],
    { encoding: "utf8" },
  );
}

function fixture() {
  const root = realpathSync(
    mkdtempSync(join(tmpdir(), "colossus-manifest-binding-")),
  );
  const executablePath = join(root, "Colossus Desktop");
  const manifestPath = join(root, "colossus-bundle-manifest.json");
  writeFileSync(manifestPath, manifest(), { mode: 0o644 });
  return { root, executablePath, manifestPath };
}

test("patches the one release placeholder with the exact manifest digest", () => {
  const { root, executablePath, manifestPath } = fixture();
  try {
    writeFileSync(executablePath, executable([placeholder]), { mode: 0o755 });
    chmodSync(executablePath, 0o755);
    const originalSize = readFileSync(executablePath).length;
    const result = run(executablePath, manifestPath);
    assert.equal(result.status, 0, result.stderr);

    const digest = createHash("sha256")
      .update(readFileSync(manifestPath))
      .digest("hex");
    const patched = readFileSync(executablePath);
    assert.equal(patched.length, originalSize);
    assert.equal(
      patched.indexOf(Buffer.from(`${prefix}${digest}${suffix}`, "ascii")) >= 0,
      true,
    );
    assert.equal(patched.indexOf(Buffer.from(placeholder, "ascii")), -1);

    const repeated = run(executablePath, manifestPath);
    assert.notEqual(repeated.status, 0);
    assert.match(repeated.stderr, /exactly one unset manifest binding/u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("patches a Windows PE image using Windows bundle names", () => {
  const { root, executablePath, manifestPath } = fixture();
  try {
    writeFileSync(
      manifestPath,
      manifest({
        targetTriple: "x86_64-pc-windows-msvc",
        sidecar: "colossus-sidecar.exe",
        cli: "colossus.exe",
      }),
      { mode: 0o644 },
    );
    writeFileSync(executablePath, portableExecutable([placeholder]), {
      mode: 0o755,
    });
    chmodSync(executablePath, 0o755);

    const result = run(executablePath, manifestPath);
    assert.equal(result.status, 0, result.stderr);
    const digest = createHash("sha256")
      .update(readFileSync(manifestPath))
      .digest("hex");
    assert.equal(
      readFileSync(executablePath).includes(
        Buffer.from(`${prefix}${digest}${suffix}`, "ascii"),
      ),
      true,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rejects missing, malformed, and duplicate release placeholders", () => {
  const { root, executablePath, manifestPath } = fixture();
  try {
    for (const bindings of [
      [],
      [`${prefix}${"0".repeat(63)}${suffix}`],
      [placeholder, placeholder],
    ]) {
      writeFileSync(executablePath, executable(bindings), { mode: 0o755 });
      chmodSync(executablePath, 0o755);
      const result = run(executablePath, manifestPath);
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /exactly one unset manifest binding/u);
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rejects non-Mach-O executables and noncanonical manifests", () => {
  const { root, executablePath, manifestPath } = fixture();
  try {
    const notMachO = executable([placeholder]);
    notMachO.writeUInt32BE(0, 0);
    writeFileSync(executablePath, notMachO, { mode: 0o755 });
    chmodSync(executablePath, 0o755);
    let result = run(executablePath, manifestPath);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /not a supported Mach-O or PE image/u);

    writeFileSync(executablePath, executable([placeholder]), { mode: 0o755 });
    chmodSync(executablePath, 0o755);
    writeFileSync(manifestPath, '{"profile":"release"}\n', { mode: 0o644 });
    result = run(executablePath, manifestPath);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /canonical release schema/u);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

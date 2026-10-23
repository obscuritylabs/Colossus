import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const wrapper = resolve(dirname(fileURLToPath(import.meta.url)), "../cargo-sccache");
const inheritedSettings = ["CARGO_INCREMENTAL", "SCCACHE_CACHE_SIZE", "SCCACHE_BASEDIRS"];

function posixShell() {
  if (process.platform !== "win32") return "sh";
  const git = spawnSync("git", ["--exec-path"], {
    encoding: "utf8", timeout: 5000, windowsHide: true,
  });
  assert.ifError(git.error);
  assert.equal(git.status, 0, git.stderr);
  for (let directory = resolve(git.stdout.trim());;) {
    for (const relative of ["bin/sh.exe", "usr/bin/sh.exe"]) {
      const candidate = join(directory, relative);
      if (existsSync(candidate)) return candidate;
    }
    const parent = dirname(directory);
    if (parent === directory) break;
    directory = parent;
  }
  throw new Error("Git for Windows must provide sh.exe for the shell wrapper fixtures");
}

function shellPath(path) {
  return process.platform === "win32"
    ? path.replaceAll("\\", "/").replace(/^([A-Za-z]):\//u, (_, drive) => `/${drive.toLowerCase()}/`)
    : path;
}

function runWrapper(t, { settings = {}, args = [], tools = ["cargo", "sccache"] } = {}) {
  const bin = mkdtempSync(join(tmpdir(), "colossus-cargo-sccache-"));
  t.after(() => rmSync(bin, { recursive: true, force: true }));
  if (tools.includes("cargo")) {
    writeFileSync(
      join(bin, "cargo"),
      `#!/bin/sh
printf '%s\\0' "$RUSTC_WRAPPER" "\${CARGO_INCREMENTAL-unset}" "\${SCCACHE_CACHE_SIZE-unset}" "\${SCCACHE_BASEDIRS-unset}" "$@"
`,
      { mode: 0o755 },
    );
  }
  if (tools.includes("sccache")) {
    writeFileSync(join(bin, "sccache"), "#!/bin/sh\nexit 71\n", { mode: 0o755 });
  }
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([name]) =>
      ![...inheritedSettings, "RUSTC_WRAPPER"].includes(name.toUpperCase()),
    ),
  );
  const result = spawnSync(
    posixShell(),
    ["-c", 'PATH=$1; export PATH; shift; exec /bin/sh "$@"', "cargo-sccache-test", shellPath(bin), shellPath(wrapper), ...args],
    { env: { ...env, ...settings }, encoding: "utf8", timeout: 5000, windowsHide: true },
  );
  assert.ifError(result.error);
  return { ...result, sccache: `${shellPath(bin)}/sccache` };
}

test("optional sccache wrapper leaves Cargo and cache defaults unset and forwards arguments", (t) => {
  const args = ["test", "-p", "colossus-cloud", "--", "a test filter", "", "$literal"];
  const result = runWrapper(t, { args });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.stdout.split("\0"), [result.sccache, "unset", "unset", "unset", ...args, ""]);
});

test("optional sccache wrapper preserves explicit incremental and cache settings", (t) => {
  for (const incremental of ["0", "1", ""]) {
    const settings = {
      CARGO_INCREMENTAL: incremental,
      SCCACHE_CACHE_SIZE: "12G",
      SCCACHE_BASEDIRS: "C:/first checkout;D:/second checkout",
    };
    const result = runWrapper(t, { settings });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(result.stdout.split("\0"), [result.sccache, ...Object.values(settings), ""]);
  }
});

test("optional sccache wrapper reports missing tools without invoking a build", (t) => {
  for (const [tools, missing] of [[[], "cargo"], [["cargo"], "sccache"]]) {
    const result = runWrapper(t, { tools });
    assert.equal(result.status, 1);
    assert.match(result.stderr, new RegExp(`${missing} is not installed or not on PATH`, "u"));
    assert.equal(result.stdout, "");
  }
});

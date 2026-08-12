import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const script = join(dirname(fileURLToPath(import.meta.url)), "render-homebrew-formula.mjs");

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "colossus-homebrew-test-"));
  const assets = join(root, "assets");
  mkdirSync(assets);
  for (const [target, digest] of [
    ["aarch64-apple-darwin", "a".repeat(64)],
    ["x86_64-apple-darwin", "b".repeat(64)],
  ]) {
    const archive = `colossus-1.2.3-${target}.tar.gz`;
    writeFileSync(join(assets, `${archive}.sha256`), `${digest}  ${archive}\n`);
  }
  return { root, assets, output: join(root, "colossus.rb") };
}

test("renders a fixed-origin two-architecture prebuilt formula", () => {
  const { assets, output } = fixture();
  execFileSync(process.execPath, [
    script,
    "--version",
    "1.2.3",
    "--assets",
    assets,
    "--output",
    output,
  ]);
  const formula = readFileSync(output, "utf8");
  assert.match(formula, /Hardware::CPU\.arm\?/u);
  assert.match(formula, /a{64}/u);
  assert.match(formula, /b{64}/u);
  assert.match(
    formula,
    /\(bin\/"colossus"\)\.write_env_script libexec\/"colossus", COLOSSUS_INSTALLER_KIND: "homebrew"/u,
  );
  assert.match(formula, /COLOSSUS_INSTALLER_KIND: "homebrew"/u);
  assert.match(formula, /colossus --version/u);
  assert.doesNotMatch(formula, /^\s*version\s+"/mu);
  assert.doesNotMatch(formula, /\n\s*bin\.write_env_script/u);
  assert.doesNotMatch(formula, /system "cargo"/u);
});

test("rejects a sidecar that names another archive", () => {
  const { assets, output } = fixture();
  const archive = "colossus-1.2.3-aarch64-apple-darwin.tar.gz";
  writeFileSync(join(assets, `${archive}.sha256`), `${"a".repeat(64)}  other.tar.gz\n`);
  assert.throws(() =>
    execFileSync(process.execPath, [
      script,
      "--version",
      "1.2.3",
      "--assets",
      assets,
      "--output",
      output,
    ], { stdio: "pipe" }),
  );
});

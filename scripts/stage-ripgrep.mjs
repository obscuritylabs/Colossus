#!/usr/bin/env node

// Stage one reviewed upstream release asset for a Colossus distribution target.
// Installation is offline; only the release build downloads this pinned archive.
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { get } from "node:https";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pin = JSON.parse(readFileSync(join(repository, "release/ripgrep.json"), "utf8"));
const MAX_ARCHIVE_BYTES = 4 * 1024 * 1024;
const REDIRECT_HOSTS = new Set(["github.com", "release-assets.githubusercontent.com"]);

function fail(message) {
  throw new Error(`stage-ripgrep: ${message}`);
}

function options(argv) {
  if (argv.length % 2 !== 0) fail("expected paired arguments");
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    if (!["--target", "--output", "--binary-name", "--archive"].includes(key)
      || values.has(key) || !argv[index + 1]) {
      fail("unknown, duplicated, or empty argument");
    }
    values.set(key, argv[index + 1]);
  }
  if (!values.has("--target") || !values.has("--output")) {
    fail("--target and --output are required");
  }
  return values;
}

function fetchArchive(url, redirects = 0) {
  if (redirects > 5 || url.protocol !== "https:" || !REDIRECT_HOSTS.has(url.hostname)) {
    fail("upstream archive redirect is outside the reviewed hosts");
  }
  return new Promise((resolveBytes, reject) => {
    const request = get(url, { timeout: 30000 }, (response) => {
      if ([301, 302, 303, 307, 308].includes(response.statusCode)) {
        const next = response.headers.location;
        response.resume();
        if (!next) return reject(new Error("upstream redirect has no location"));
        fetchArchive(new URL(next, url), redirects + 1).then(resolveBytes, reject);
        return;
      }
      if (response.statusCode !== 200) {
        response.resume();
        return reject(new Error(`upstream returned HTTP ${response.statusCode}`));
      }
      const chunks = [];
      let length = 0;
      response.on("data", (chunk) => {
        length += chunk.length;
        if (length > MAX_ARCHIVE_BYTES) {
          request.destroy(new Error("upstream archive exceeds size limit"));
        } else {
          chunks.push(chunk);
        }
      });
      response.on("end", () => resolveBytes(Buffer.concat(chunks)));
      response.on("error", reject);
    });
    request.on("timeout", () => request.destroy(new Error("upstream archive timed out")));
    request.on("error", reject);
  });
}

const values = options(process.argv.slice(2));
const target = values.get("--target");
const expected = pin.archives[target];
if (!expected || !/^[a-f0-9]{64}$/.test(expected)) fail("unsupported target or invalid pin");
const windows = target.includes("-windows-");
const archiveName = `ripgrep-${pin.version}-${target}.${windows ? "zip" : "tar.gz"}`;
const binaryName = values.get("--binary-name") ?? (windows ? "rg.exe" : "rg");
if (basename(binaryName) !== binaryName || !/^rg(?:-[A-Za-z0-9_.-]+)?(?:\.exe)?$/.test(binaryName)) {
  fail("binary name is invalid");
}
const output = resolve(values.get("--output"));
const localArchive = values.get("--archive");
const bytes = localArchive
  ? readFileSync(resolve(localArchive))
  : await fetchArchive(new URL(`https://github.com/BurntSushi/ripgrep/releases/download/${pin.version}/${archiveName}`));
if (bytes.length === 0 || bytes.length > MAX_ARCHIVE_BYTES) fail("archive size is invalid");
const actual = createHash("sha256").update(bytes).digest("hex");
if (actual !== expected) fail(`archive checksum mismatch for ${target}`);

const temporary = mkdtempSync(join(tmpdir(), "colossus-ripgrep-"));
try {
  const archive = join(temporary, archiveName);
  writeFileSync(archive, bytes, { mode: 0o600 });
  const extracted = join(temporary, "extracted");
  mkdirSync(extracted);
  const arguments_ = windows
    ? ["-xf", archive, "-C", extracted]
    : ["-xzf", archive, "-C", extracted];
  const result = spawnSync("tar", arguments_, { stdio: "pipe", timeout: 30000 });
  if (result.status !== 0) fail("verified archive could not be extracted");
  const root = join(extracted, `ripgrep-${pin.version}-${target}`);
  const executable = join(root, windows ? "rg.exe" : "rg");
  for (const name of [windows ? "rg.exe" : "rg", "COPYING", "LICENSE-MIT", "UNLICENSE"]) {
    if (!existsSync(join(root, name))) fail(`verified archive lacks ${name}`);
  }
  mkdirSync(output, { recursive: true });
  copyFileSync(executable, join(output, binaryName));
  chmodSync(join(output, binaryName), 0o755);
  for (const notice of ["COPYING", "LICENSE-MIT", "UNLICENSE"]) {
    const destination = join(output, notice);
    copyFileSync(join(root, notice), destination);
    chmodSync(destination, 0o644);
  }
  process.stdout.write(`staged ripgrep ${pin.version} for ${target} (${actual})\n`);
} finally {
  rmSync(temporary, { recursive: true, force: true });
}

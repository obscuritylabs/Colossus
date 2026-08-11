#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";

function fail(message) {
  throw new Error(`homebrew formula: ${message}`);
}

function argumentsFrom(argv) {
  const parsed = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined || parsed.has(name)) {
      fail("expected unique --version, --assets, and --output arguments");
    }
    parsed.set(name, value);
  }
  for (const required of ["--version", "--assets", "--output"]) {
    if (!parsed.has(required)) fail(`missing ${required}`);
  }
  if (parsed.size !== 3) fail("unknown argument");
  return Object.fromEntries(parsed);
}

function checksum(assets, archive) {
  const path = join(assets, `${archive}.sha256`);
  const bytes = readFileSync(path);
  if (bytes.length > 512) fail(`${basename(path)} exceeds the fixed limit`);
  const match = /^([0-9a-f]{64})  ([A-Za-z0-9._-]+)\n?$/.exec(bytes.toString("ascii"));
  if (!match || match[2] !== archive) fail(`${basename(path)} has an invalid shape`);
  return match[1];
}

function render(version, armSha, intelSha) {
  return `class Colossus < Formula
  desc "Auditable runtime for agent work and durable automation"
  homepage "https://github.com/obscuritylabs/Colossus"
  version "${version}"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/obscuritylabs/Colossus/releases/download/v${version}/colossus-${version}-aarch64-apple-darwin.tar.gz"
      sha256 "${armSha}"
    else
      url "https://github.com/obscuritylabs/Colossus/releases/download/v${version}/colossus-${version}-x86_64-apple-darwin.tar.gz"
      sha256 "${intelSha}"
    end
  end

  def install
    libexec.install "colossus"
    bin.write_env_script libexec/"colossus", COLOSSUS_INSTALLER_KIND: "homebrew"
  end

  test do
    assert_equal "colossus #{version}", shell_output("#{bin}/colossus --version").strip
  end
end
`;
}

const options = argumentsFrom(process.argv.slice(2));
const version = options["--version"];
if (!/^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(version)) {
  fail("version must be an exact stable X.Y.Z value");
}
const armArchive = `colossus-${version}-aarch64-apple-darwin.tar.gz`;
const intelArchive = `colossus-${version}-x86_64-apple-darwin.tar.gz`;
const formula = render(
  version,
  checksum(options["--assets"], armArchive),
  checksum(options["--assets"], intelArchive),
);
writeFileSync(options["--output"], formula, { encoding: "utf8", flag: "wx" });

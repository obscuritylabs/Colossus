import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { checkToolchain } from './check-toolchain.mjs';

const root = fileURLToPath(new URL('../..', import.meta.url));
const files = ['mise.toml', 'mise.lock', 'mise.devcontainer.toml', 'mise.devcontainer.lock', 'mise.fuzz.toml', 'mise.fuzz.lock', 'scripts/ci/mise-bootstrap.sh', 'rust-toolchain.toml', 'Dockerfile', 'deploy/cloud/linux-desktop.Dockerfile', '.devcontainer', '.github/workflows', '.github/actions/setup-toolchain'];

test('repository provisioning agrees with the inventory', () => {
  assert.deepEqual(checkToolchain(root), []);
});

test('CLI resolves the repository independently of the caller directory', () => {
  const result = spawnSync(process.execPath, [resolve(root, 'scripts/ci/check-toolchain.mjs')], { cwd: tmpdir(), encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
});

for (const [name, file, before, after, diagnostic] of [
  ['retained workflow pin', '.github/workflows/release.yml', 'node-version: 22.18.0', 'node-version: 23.0.0', 'inventory requires'],
  ['Linux Desktop image pin', 'deploy/cloud/linux-desktop.Dockerfile', 'FROM node:22.18.0-', 'FROM node:22.19.0-', 'node image'],
  ['container pin', '.devcontainer/Dockerfile', 'FROM rust:1.96.0-', 'FROM rust:1.95.0-', 'rust image'],
  ['container context', '.devcontainer/devcontainer.json', '"context": ".."', '"context": "."', 'build context'],
  ['fuzz pin', 'mise.fuzz.lock', 'nightly-2026-07-10', 'nightly-2026-07-11', 'nightly'],
  ['stale lock', 'mise.toml', 'node = "22.18.0"', 'node = "22.19.0"', 'mise.lock node'],
  ['missing checksum', 'mise.lock', 'checksum = "sha256:', 'removed_checksum = "sha256:', 'missing verified'],
  ['compiler components', 'rust-toolchain.toml', '["clippy", "rustfmt"]', '["clippy", "rustfmt", "rust-src"]', 'Rust components'],
  ['audit source verification', 'mise.toml', 'cargo.binstall = false', 'cargo.binstall = true', 'native source builds'],
  ['backend drift', 'mise.toml', 'node = "core:node"', 'node = "asdf:node"', 'mise backend node'],
  ['missing executable checksum', 'scripts/ci/mise-bootstrap.sh', 'sha256=e79ae', 'removed_sha256=e79ae', 'verified Linux:X64'],
  ['container environment discovery', '.devcontainer/Dockerfile', 'MISE_CONFIG_DIR=/opt/colossus-toolchain', 'MISE_GLOBAL_CONFIG_FILE=/opt/colossus-toolchain/mise.toml', 'MISE_CONFIG_DIR'],
  ['unlocked container install', '.devcontainer/Dockerfile', 'mise install --locked', 'mise install', 'mise install --locked'],
  ['reintroduced Rust feature', '.devcontainer/devcontainer.json', '"features": {', '"features": {"ghcr.io/devcontainers/features/rust:1": {"version": "1.96.0"},', 'Rust must consume'],
  ['unlocked fuzz lock', 'mise.fuzz.lock', 'locked = "true"', 'locked = "false"', 'must use Cargo.lock'],
  ['fuzz profile', 'mise.fuzz.lock', 'profile = "minimal"', 'profile = "default"', 'Rust profile'],
  ['container audit compatibility', 'mise.devcontainer.toml', 'os = ["linux", "macos"]', 'os = ["macos"]', 'compatible locked source builds'],
  ['container audit source lock', 'mise.devcontainer.lock', 'locked = "true"', 'locked = "false"', 'source build must use Cargo.lock'],
  ['unlocked fuzz build', 'mise.fuzz.toml', 'locked = true', 'locked = false', 'locked Cargo'],
  ['reintroduced premerge installer', '.github/workflows/premerge.yml', 'uses: ./.github/actions/setup-toolchain', 'uses: actions/setup-node@deadbeef', 'central inventory'],
  ['unlocked install', '.github/actions/setup-toolchain/action.yml', 'install_args: --locked', 'install_args:', 'locked selective'],
  ['reintroduced PR installer', '.github/workflows/pr.yml', 'uses: ./.github/actions/setup-toolchain', 'uses: actions/setup-node@deadbeef', 'central inventory'],
]) {
  test(`rejects ${name}`, () => {
    const fixture = mkdtempSync(resolve(tmpdir(), 'colossus-toolchain-'));
    try {
      for (const path of files) cpSync(resolve(root, path), resolve(fixture, path), { recursive: true });
      const path = resolve(fixture, file);
      const original = readFileSync(path, 'utf8');
      assert.ok(original.includes(before), 'mutation must change its intended fixture');
      writeFileSync(path, original.replace(before, after));
      assert.ok(checkToolchain(fixture).some((error) => error.includes(diagnostic)));
    } finally {
      rmSync(fixture, { recursive: true, force: true });
    }
  });
}

for (const [os, arch] of [['Linux', 'X64'], ['Linux', 'ARM64'], ['macOS', 'ARM64'], ['Windows', 'X64']]) {
  test(`bootstrap selects verified ${os}:${arch}`, () => {
    const result = spawnSync('sh', [resolve(root, 'scripts/ci/mise-bootstrap.sh'), os, arch], { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /^version=\d{4}\.\d+\.\d+\nsha256=[a-f0-9]{64}\n$/);
  });
}

test('bootstrap rejects unsupported platforms', () => {
  const result = spawnSync('sh', [resolve(root, 'scripts/ci/mise-bootstrap.sh'), 'macOS', 'X64'], { encoding: 'utf8' });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Unsupported mise platform/);
});

test('container bootstrap rejects corrupt download bytes before installation', {
  skip: process.platform !== 'linux' && 'Linux installer uses GNU sha256sum',
}, () => {
  const fixture = mkdtempSync(resolve(tmpdir(), 'colossus-mise-download-'));
  try {
    const curl = resolve(fixture, 'curl');
    writeFileSync(curl, `#!/bin/sh
while [ "$#" -gt 0 ]; do
  if [ "$1" = --output ]; then
    printf 'corrupt download' > "$2"
    exit 0
  fi
  shift
done
exit 1
`);
    chmodSync(curl, 0o755);
    const destination = resolve(fixture, 'mise');
    const result = spawnSync('sh', [resolve(root, 'scripts/ci/mise-bootstrap.sh'), 'Linux', 'X64', '--install', destination], {
      encoding: 'utf8', env: { ...process.env, PATH: `${fixture}:${process.env.PATH}` },
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stdout + result.stderr, /FAILED|did NOT match/);
    assert.equal(existsSync(destination), false);
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
});

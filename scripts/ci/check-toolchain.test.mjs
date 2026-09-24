import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { checkToolchain } from './check-toolchain.mjs';

const root = fileURLToPath(new URL('../..', import.meta.url));
const files = ['mise.toml', 'mise.lock', 'rust-toolchain.toml', 'Dockerfile', '.devcontainer', '.github/workflows', '.github/actions/setup-toolchain'];

test('repository provisioning agrees with the inventory', () => {
  assert.deepEqual(checkToolchain(root), []);
});

test('CLI resolves the repository independently of the caller directory', () => {
  const result = spawnSync(process.execPath, [resolve(root, 'scripts/ci/check-toolchain.mjs')], { cwd: tmpdir(), encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
});

for (const [name, file, before, after, diagnostic] of [
  ['workflow pin', '.github/workflows/premerge.yml', 'node-version: 22.18.0', 'node-version: 23.0.0', 'inventory requires'],
  ['container pin', '.devcontainer/Dockerfile', 'FROM python:3.10.18-', 'FROM python:3.12.0-', 'python image'],
  ['feature pin', '.devcontainer/devcontainer.json', '"version": "1.96.0"', '"version": "1.95.0"', 'Rust feature'],
  ['fuzz command', '.github/workflows/premerge.yml', 'cargo +nightly-2026-07-10', 'cargo +nightly-2026-07-11', 'nightly'],
  ['stale lock', 'mise.toml', 'node = "22.18.0"', 'node = "22.19.0"', 'mise.lock node'],
  ['missing checksum', 'mise.lock', 'checksum = "sha256:', 'removed_checksum = "sha256:', 'missing verified'],
  ['compiler components', 'rust-toolchain.toml', '["clippy", "rustfmt"]', '["clippy", "rustfmt", "rust-src"]', 'Rust components'],
  ['audit source verification', 'mise.toml', 'cargo.binstall = false', 'cargo.binstall = true', 'native source builds'],
  ['backend drift', 'mise.toml', 'node = "core:node"', 'node = "asdf:node"', 'mise backend node'],
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

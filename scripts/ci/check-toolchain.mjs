#!/usr/bin/env node
// This reads only the repository's scalar pin fields, not arbitrary TOML/YAML.
// Unsupported/missing declarations fail closed instead of guessing a version.
import { readFileSync, readdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const escape = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
function scalar(text, section, key) {
  const body = text.split(`[${section}]`)[1]?.split(/^\[/m)[0];
  const matches = [...(body ?? '').matchAll(new RegExp(`^"?${escape(key)}"?\\s*=\\s*"([^"\\n]+)"\\s*(?:#.*)?$`, 'gm'))];
  if (matches.length !== 1) throw new Error(`expected one scalar ${section}.${key}`);
  return matches[0][1];
}

export function checkToolchain(root) {
  const read = (path) => readFileSync(resolve(root, path), 'utf8');
  const errors = [];
  const equal = (where, actual, expected) => {
    if (actual !== expected) errors.push(`${where}: found ${actual}; inventory requires ${expected}`);
  };
  const config = read('mise.toml');
  const rust = read('rust-toolchain.toml');
  const tools = {
    node: scalar(config, 'tools', 'node'),
    python: scalar(config, 'tools', 'python'),
    go: scalar(config, 'tools', 'go'),
    actionlint: scalar(config, 'tools', 'aqua:rhysd/actionlint'),
    'cargo-deny': scalar(config, 'tools', 'aqua:EmbarkStudios/cargo-deny'),
    'cargo-audit': scalar(config, 'vars', 'cargo_audit'),
    rust: scalar(rust, 'toolchain', 'channel'),
    nightly: scalar(config, 'vars', 'fuzz_rust'),
    'cargo-fuzz': scalar(config, 'vars', 'cargo_fuzz'),
    npm: scalar(config, 'vars', 'release_npm'),
  };
  const scan = (path, pattern, key, group = 1) => {
    for (const match of read(path).matchAll(pattern)) {
      equal(`${path} (${key})`, match[group], tools[key]);
    }
  };
  for (const name of readdirSync(resolve(root, '.github/workflows')).filter((p) => /\.ya?ml$/.test(p))) {
    const path = `.github/workflows/${name}`;
    for (const [field, key] of [['node-version', 'node'], ['python-version', 'python'], ['go-version', 'go']]) {
      scan(path, new RegExp(`^\\s*${field}:\\s*["']?([^\\s"'#]+)`, 'gm'), key);
    }
    for (const match of read(path).matchAll(/^\s*toolchain:\s*["']?([^\s"'#]+)/gm)) {
      equal(`${path} (rust)`, match[1], tools[match[1].startsWith('nightly') ? 'nightly' : 'rust']);
    }
    scan(path, /cargo \+(nightly-[\d-]+)/g, 'nightly');
    for (const tool of ['cargo-deny', 'cargo-audit', 'cargo-fuzz', 'npm']) {
      scan(path, new RegExp(`${tool}(?:@| --version )([\\d.]+)`, 'g'), tool);
    }
  }
  for (const path of ['Dockerfile', '.devcontainer/Dockerfile']) {
    for (const match of read(path).matchAll(/^FROM (node|python|golang|rust):([\d.]+)/gm)) {
      const key = match[1] === 'golang' ? 'go' : match[1];
      // The root utility image deliberately uses the stable Rust minor series.
      const expected = path === 'Dockerfile' && key === 'rust'
        ? tools.rust.split('.').slice(0, 2).join('.') : tools[key];
      equal(`${path} (${key} image)`, match[2], expected);
    }
    scan(path, /actionlint@v([\d.]+)/g, 'actionlint');
    for (const tool of ['cargo-deny', 'cargo-audit']) {
      scan(path, new RegExp(`${tool} --version ([\\d.]+)`, 'g'), tool);
    }
  }
  const features = JSON.parse(read('.devcontainer/devcontainer.json')).features;
  const feature = Object.entries(features).find(([key]) => key.startsWith('ghcr.io/devcontainers/features/rust:'))?.[1];
  equal('.devcontainer Rust feature', feature?.version, tools.rust);
  equal('.devcontainer Rust profile', feature?.profile, scalar(rust, 'toolchain', 'profile'));
  const components = rust.match(/^components\s*=\s*\[([^\]]+)\]/m)?.[1].match(/"([^"]+)"/g)?.map((v) => v.slice(1, -1));
  if (!components?.length) errors.push('missing required Rust components');
  for (const component of components ?? []) {
    if (!feature?.components?.split(',').includes(component)) errors.push(`devcontainer missing ${component}`);
  }

  for (const declaration of [
    '"aqua:rustsec/rustsec/cargo-audit" = { version = "{{ vars.cargo_audit }}", os = ["linux", "windows"] }',
    '"cargo:cargo-audit" = { version = "{{ vars.cargo_audit }}", os = ["macos"], locked = true }',
    'cargo.binstall = false',
  ]) {
    if (!config.includes(declaration)) errors.push('audit platform backends must retain locked native source builds on macOS');
  }
  for (const tool of ['node', 'python', 'go', 'rust']) {
    equal(`mise backend ${tool}`, scalar(config, 'tool_alias', tool), `core:${tool}`);
  }
  const lock = read('mise.lock');
  const locked = new Map();
  for (const block of lock.split(/^\[\[tools\./m).slice(1)) {
    const name = block.slice(0, block.indexOf(']]')).replaceAll('"', '');
    locked.set(name, block);
  }
  const backends = {
    node: 'core:node', python: 'core:python', go: 'core:go', rust: 'core:rust',
    'aqua:rhysd/actionlint': 'aqua:rhysd/actionlint',
    'aqua:EmbarkStudios/cargo-deny': 'aqua:EmbarkStudios/cargo-deny',
    'aqua:rustsec/rustsec/cargo-audit': 'aqua:rustsec/rustsec/cargo-audit',
    'cargo:cargo-audit': 'cargo:cargo-audit',
  };
  for (const [name, backend] of Object.entries(backends)) {
    const block = locked.get(name) ?? '';
    const key = name === 'cargo:cargo-audit' ? 'cargo-audit' : name.includes(':') ? name.split('/').at(-1) : name;
    equal(`mise.lock ${name}`, block.match(/^version = "([^"]+)"/m)?.[1], tools[key]);
    equal(`mise.lock ${name} backend`, block.match(/^backend = "([^"]+)"/m)?.[1], backend);
    if (name === 'cargo:cargo-audit') {
      if (!/^locked = "true"$/m.test(block)) errors.push('macOS audit source build must use Cargo.lock');
      continue;
    }
    if (name === 'rust') {
      equal('mise.lock Rust profile', block.match(/^profile = "([^"]+)"/m)?.[1], scalar(rust, 'toolchain', 'profile'));
      equal('mise.lock Rust components', block.match(/^components = "([^"]+)"/m)?.[1], components?.join(','));
      continue; // rustup verifies compiler distributions; no mise archive URL.
    }
    for (const platform of ['linux-x64', 'macos-arm64', 'windows-x64']) {
      const entry = block.split(`"platforms.${platform}"]`)[1]?.split(/^\[/m)[0] ?? '';
      if (!/^checksum = "sha256:[a-f0-9]{64}"$/m.test(entry) || !/^url = "https:\/\/[^"\s]+"$/m.test(entry)) {
        errors.push(`mise.lock ${name}: missing verified ${platform} archive`);
      }
    }
  }
  const pr = read('.github/workflows/pr.yml');
  if (/uses: (?:actions\/setup-(?:node|python|go)|dtolnay\/rust-toolchain|taiki-e\/install-action)@|ACTIONLINT_VERSION/.test(pr)) {
    errors.push('PR provisioning must consume the central inventory');
  }
  const setup = read('.github/actions/setup-toolchain/action.yml');
  if (!/uses: jdx\/mise-action@[a-f0-9]{40}\b/.test(setup) ||
      !/^\s+version: \d{4}\.\d+\.\d+$/m.test(setup) ||
      !/^\s+sha256: [a-f0-9]{64}$/m.test(setup) ||
      !/install_args: --locked \$\{\{ inputs.tools }}/.test(setup)) {
    errors.push('mise bootstrap must pin action, executable, checksum and locked selective installation');
  }
  return errors;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const errors = checkToolchain(process.argv[2] ?? process.cwd());
    if (errors.length) throw new Error(errors.join('\n'));
    console.log('Toolchain inventory, retained consumers and platform locks agree.');
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

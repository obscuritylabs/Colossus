#!/bin/sh

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

expect_classification() {
    expected=$1
    shift
    actual=$($script_dir/classify-changes.sh "$@")
    if [ "$actual" != "$expected" ]; then
        printf 'unexpected classification for %s\nexpected:\n%s\nactual:\n%s\n' "$*" "$expected" "$actual" >&2
        exit 1
    fi
}

expect_classification 'rust_required=false
docs_required=true
dependency_required=false
sdk_required=false
desktop_required=false' docs/index.md README.md
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=false' crates/colossus-runtime/src/lib.rs
expect_classification 'rust_required=true
docs_required=true
dependency_required=false
sdk_required=false
desktop_required=false' docs/index.md crates/colossus-runtime/src/lib.rs
expect_classification 'rust_required=true
docs_required=false
dependency_required=true
sdk_required=false
desktop_required=false' Cargo.lock
expect_classification 'rust_required=true
docs_required=false
dependency_required=true
sdk_required=false
desktop_required=false' crates/colossus-runtime/Cargo.toml
expect_classification 'rust_required=true
docs_required=false
dependency_required=true
sdk_required=false
desktop_required=true' apps/desktop/package-lock.json
expect_classification 'rust_required=true
docs_required=false
dependency_required=true
sdk_required=true
desktop_required=false' sdk/go/go.sum
expect_classification 'rust_required=true
docs_required=false
dependency_required=true
sdk_required=true
desktop_required=false' sdk/python/pyproject.toml sdk/python/requirements-dev.txt
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=true
desktop_required=false' api/colossus/api/v1alpha1/agent_run.proto sdk/typescript/src/index.ts
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=true
desktop_required=false' apps/vscode/src/extension.ts
expect_classification 'rust_required=true
docs_required=false
dependency_required=true
sdk_required=true
desktop_required=false' apps/vscode/package-lock.json

expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=true
desktop_required=true' apps/ui/src/components/Composer.tsx

expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' apps/desktop/src/App.tsx release/ripgrep.json scripts/desktop-dev scripts/package-desktop-macos scripts/package-desktop-windows.ps1 scripts/stage-ripgrep.mjs scripts/patch-desktop-manifest-binding.mjs scripts/prepare-desktop-binaries scripts/write-desktop-bundle-manifest.mjs scripts/verify-desktop-bundle.mjs scripts/verify-desktop-unsigned-archive.mjs crates/colossus-sdk/src/lib.rs crates/colossus-sidecar/src/main.rs crates/colossus-sidecar-protocol/src/lib.rs
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' crates/colossus-cli/src/main.rs
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' scripts/desktop-chromium-preview
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' scripts/open-desktop-chromium-preview
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' scripts/desktop-chromium-preview-windows.ps1
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' scripts/browser-native-host-windows.ps1
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' apps/web/src/App.tsx
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' apps/control-plane-client/src/index.ts
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' crates/colossus-darwin-process/src/lib.rs

# Check each native browser owner independently; a Desktop file in the same
# change set must not conceal missing native shim, component, or harness coverage.
for native_browser_path in \
    native/browser/CMakeLists.txt native/browser/README.md \
    native/browser/include/colossus_cef.h native/browser/src/client.cc \
    native/browser/src/platform_mac.mm native/browser/component/cef.lock.json \
    native/browser/scripts/component.py native/browser/scripts/run_probe.py \
    native/browser/scripts/test_component.py native/browser/scripts/test_probe.py \
    native/browser/tests/fixture.html \
    native/browser/src/windows_entry.cc native/browser/src/platform_win.cc \
    native/browser/scripts/stage_windows.py native/browser/scripts/test_stage_windows.py \
    native/browser/scripts/windows_process_evidence.ps1 \
    native/browser/scripts/run_host_probe.py native/browser/scripts/test_host_probe.py \
    native/browser/scripts/host_capture_probe.py native/browser/scripts/test_host_capture_probe.py \
    native/browser/scripts/host_transfer_probe.py native/browser/scripts/test_host_transfer_probe.py \
    native/browser/scripts/elf_order.py native/browser/scripts/test_elf_order.py \
    native/browser/scripts/run_host_pki_probe.py native/browser/scripts/pki_fixture_proxy.py \
    native/browser/scripts/pki_fixture.py native/browser/scripts/pki_fixture_material.py \
    native/browser/scripts/pki_fixture_private.py native/browser/scripts/test_pki_fixture.py \
    native/browser/scripts/pki_fixture_retirement.py native/browser/scripts/test_pki_fixture_retirement.py \
    native/browser/scripts/run_renderer_custody_probe.py native/browser/scripts/test_renderer_custody_probe.py \
    native/browser/scripts/stage_nss_tools.py native/browser/nss-tools.lock.json \
    native/browser/scripts/test_stage_nss_tools.py \
    native/browser/driver/src/main.rs native/browser/pki/src/lib.rs
do
    expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' "$native_browser_path"
done

for presentation_path in crates/colossus-browser-presentation/src/frame.rs \
    crates/colossus-browser-presentation/src/lease.rs \
    crates/colossus-windows-native/src/windows/appcontainer_principal.rs \
    crates/colossus-windows-process/src/private_io.rs
do
    expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' "$presentation_path"
done

expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=false' native/browser-other/component.py

# Check each standalone path independently: a simultaneous CLI change must not
# conceal a missing native-computer classifier owner.
for native_computer_path in \
    native/computer/src/lib.rs native/computer/src/platform/capture.rs \
    native/computer/examples/computer_window_fixture.rs \
    native/computer/examples/computer_acceptance.rs native/computer/README.md
do
    expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=true' "$native_computer_path"
done
for native_computer_manifest in native/computer/Cargo.toml native/computer/Cargo.lock
do
    expect_classification 'rust_required=true
docs_required=false
dependency_required=true
sdk_required=false
desktop_required=true' "$native_computer_manifest"
done
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=false' native/computer-other/src/lib.rs

expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=true
desktop_required=true' .github/workflows/pr.yml scripts/ci/classify-changes.sh crates/colossus-cli/tests/ci_contract.rs
expect_classification 'rust_required=true
docs_required=false
dependency_required=true
sdk_required=true
desktop_required=true' xtask/src/checks/sdk.rs xtask/src/checks/surfaces.rs
expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=false' unknown/new-boundary.file
expect_classification 'rust_required=false
docs_required=true
dependency_required=false
sdk_required=false
desktop_required=false' docs/renamed.md
expect_classification 'rust_required=false
docs_required=true
dependency_required=false
sdk_required=false
desktop_required=false' docs/deleted.md

# Exercise each path independently so another changed file cannot hide a
# missing selector. Classification is path-based for additions, edits and deletes.
for shared_toolchain_path in \
    mise.toml mise.lock mise.ci.toml .mise.toml .mise.lock .mise.ci.toml \
    .mise/config.toml .mise/tasks/docs/build mise-tasks/docs/build \
    .github/actions/setup/action.yml .github/actions/setup/install.sh \
    .devcontainer/Dockerfile .devcontainer/devcontainer.json \
    .devcontainer/devcontainer-lock.json rust-toolchain rust-toolchain.toml
do
    expect_classification 'rust_required=true
docs_required=true
dependency_required=true
sdk_required=true
desktop_required=true' "$shared_toolchain_path"
done

expect_classification 'rust_required=false
docs_required=true
dependency_required=false
sdk_required=false
desktop_required=false' CLAUDE.md

expect_classification 'rust_required=true
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=false' unrelated/mise.toml

if $script_dir/classify-changes.sh >/dev/null 2>&1; then
    printf 'empty change classification unexpectedly succeeded\n' >&2
    exit 1
fi

$script_dir/require-pr-results.sh success true success false skipped false skipped
$script_dir/require-pr-results.sh success false skipped true success false skipped
$script_dir/require-success.sh rust=success windows=success macos=success

for invalid in \
    'failure true success false skipped false skipped' \
    'success true failure false skipped false skipped' \
    'success false success true success false skipped' \
    'success false skipped false skipped true cancelled'
do
    # Intentional field splitting exercises the positional shell interface.
    # shellcheck disable=SC2086
    if $script_dir/require-pr-results.sh $invalid >/dev/null 2>&1; then
        printf 'invalid PR result set unexpectedly succeeded: %s\n' "$invalid" >&2
        exit 1
    fi
done

if $script_dir/require-success.sh rust=success windows=cancelled >/dev/null 2>&1; then
    printf 'cancelled result unexpectedly satisfied the aggregate gate\n' >&2
    exit 1
fi

if $script_dir/require-success.sh eligibility=skipped >/dev/null 2>&1; then
    printf 'skipped eligibility unexpectedly satisfied the pre-merge gate\n' >&2
    exit 1
fi

# Python 3.10.18 has Ubuntu 24.04 builds, but none for the newer floating image.
"${NODE:-node}" --input-type=module - "$script_dir/../../.github/workflows/pr.yml" "$script_dir/../../.github/workflows/premerge.yml" <<'NODE'
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

for (const [path, job] of [[process.argv[2], 'sdk'], [process.argv[3], 'vscode-sdk']]) {
  const lines = readFileSync(path, 'utf8').replaceAll('\r\n', '\n').split('\n');
  const start = lines.indexOf('  ' + job + ':');
  assert.ok(start >= 0, 'Missing SDK workflow job: ' + job);
  const end = lines.findIndex((line, index) => index > start && /^  [\w-]+:$/u.test(line));
  const scope = lines.slice(start, end < 0 ? lines.length : end).join('\n');
  if (/^          python-version: 3\.10\.18$/mu.test(scope)) {
    assert.match(scope, /^    runs-on: ubuntu-24\.04$/mu,
      job + ' must use an image with the pinned Python 3.10.18 distribution');
  }
}
NODE

# rust-cache uses shared-key instead of key, so the bundle recipe must be part
# of the effective shared namespace in the trusted producer and every consumer.
"${NODE:-node}" --input-type=module - "$script_dir/../../.github/workflows/cache-warm.yml" "$script_dir/../../.github/workflows/premerge.yml" "$script_dir/../../.github/workflows/release.yml" <<'NODE'
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

function block(lines, title, sibling) {
  const start = lines.indexOf(title);
  assert.ok(start >= 0, 'Missing workflow scope: ' + title.trim());
  const end = lines.findIndex((line, index) => index > start && sibling.test(line));
  return lines.slice(start, end < 0 ? lines.length : end);
}

const namespaces = [];
for (const [path, job, step] of [
  [process.argv[2], 'macos-desktop-bundle', 'Save macOS Desktop bundle build cache'],
  [process.argv[3], 'macos-desktop-bundle', 'Restore main Desktop bundle build cache'],
  [process.argv[4], 'desktop_macos_build', 'Restore main Desktop bundle build cache'],
]) {
  const source = readFileSync(path, 'utf8').replaceAll('\r\n', '\n').split('\n');
  const jobLines = block(source, '  ' + job + ':', /^  [\w-]+:$/u);
  const cache = block(jobLines, '      - name: ' + step, /^      - /u).join('\n');
  const namespace = cache.match(/^          shared-key: (\S+)$/mu)?.[1];
  assert.equal(namespace, 'macos-15-desktop-bundle-recipe-v3',
    job + ' must version the effective shared cache namespace');
  assert.doesNotMatch(cache, /^          key:/mu,
    job + ' must not rely on a key ignored by shared-key');
  namespaces.push(namespace);
}
assert.equal(new Set(namespaces).size, 1,
  'The bundle cache producer and both consumers must share an effective namespace');
NODE

# Keep the compiler-output mitigation local to Windows Desktop and its trusted
# cache producer, and fail before Cargo if the exact compiler cannot be resolved.
"${NODE:-node}" --input-type=module - "$script_dir/../../.github/workflows/premerge.yml" "$script_dir/../../.github/workflows/sccache-warm.yml" <<'NODE'
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const workflow = readFileSync(process.argv[2], 'utf8').replaceAll('\r\n', '\n');
const start = workflow.indexOf('\n  windows-desktop:\n');
const end = workflow.indexOf('\n  fuzz:\n', start);
assert.ok(start >= 0 && end > start, 'Windows Desktop job must be independently scoped');
const windows = workflow.slice(start, end);
const outside = workflow.slice(0, start) + workflow.slice(end);
assert.doesNotMatch(outside, /\bclang-cl(?:\.exe)?\b|^\s+(?:CC|CXX):/mu);

const selection = windows.indexOf('Get-Command clang-cl.exe');
const probeStart = windows.lastIndexOf('\n      - name:', selection);
const probeEnd = windows.indexOf('\n      - name:', probeStart + 1);
const firstCargo = windows.search(/\bcargo\s/u);
assert.ok(selection >= 0 && probeStart >= 0 && probeEnd > selection && probeEnd < firstCargo,
  'Compiler prerequisite must run before the first Cargo command');
const probe = windows.slice(probeStart, probeEnd);
assert.match(probe, /shell: pwsh\n/u);
assert.match(probe, /timeout-minutes: 1\n/u);
assert.doesNotMatch(probe, /continue-on-error:|\n        if:/u);
assert.match(probe, /\$ErrorActionPreference = 'Stop'/u);
const resolved = probe.match(/\$(\w+)\s*=\s*\(Get-Command clang-cl\.exe\b[^\n]*-CommandType Application\b[^\n]*-ErrorAction Stop\)\.(?:Source|Path)/u);
assert.ok(resolved, 'Resolve an executable application path without an alias or fallback');
const compiler = '\\$' + resolved[1];
const version = probe.search(new RegExp('&\\s+' + compiler + '\\s+--version', 'u'));
const checked = probe.search(/if\s*\(\s*\$LASTEXITCODE\s*-ne\s*0\s*\)\s*\{\s*exit\s+\$LASTEXITCODE\s*\}/u);
const exported = ['CC', 'CXX'].map(name =>
  probe.search(new RegExp(`["']${name}=${compiler}["']`, 'u')));
const destination = probe.search(/Add-Content\b[^\n]*-LiteralPath\s+\$env:GITHUB_ENV\b/u);
assert.ok(version >= 0 && checked > version && exported.every(index => index > checked) &&
  destination > Math.max(...exported),
  'Both compiler paths must be exported only after a successful version probe');
assert.doesNotMatch(windows, /CMAKE_(?:C_COMPILER|CXX_COMPILER|GENERATOR_TOOLSET)|-T\s*(?:ClangCL|LLVM)/u);

const warmer = readFileSync(process.argv[3], 'utf8').replaceAll('\r\n', '\n');
const warmSelection = warmer.indexOf('Get-Command clang-cl.exe');
const warmStart = warmer.lastIndexOf('\n      - name:', warmSelection);
const warmEnd = warmer.indexOf('\n      - name:', warmStart + 1);
assert.ok(warmSelection >= 0 && warmEnd > warmSelection && warmEnd < warmer.search(/\bcargo\s/u),
  'The Windows cache producer must select the compiler before Cargo');
const warmProbe = warmer.slice(warmStart, warmEnd);
assert.match(warmProbe, /\n        if: runner\.os == 'Windows'\n/u);
assert.match(warmProbe, /shell: pwsh\n/u);
assert.match(warmProbe, /timeout-minutes: 1\n/u);
assert.doesNotMatch(warmProbe, /continue-on-error:/u);
const commandBody = value => value.slice(value.indexOf('\n        run: |\n'));
assert.equal(commandBody(warmProbe), commandBody(probe),
  'The Windows cache producer and Desktop consumer must use the same verified compiler');
NODE

# The optional R2 backend must not replace the GitHub read-only fallback when
# fork PRs have no secrets, and must never write with a read credential.
r2_env_file=$(mktemp)
trap 'rm -f "$r2_env_file"' EXIT HUP INT TERM
GITHUB_ENV=$r2_env_file "$script_dir/configure-sccache-r2.sh" read >/dev/null
test ! -s "$r2_env_file"
R2_BUCKET=colossus-sccache \
R2_ENDPOINT=https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com \
R2_REGION=auto R2_ACCESS_KEY_ID=test-read R2_SECRET_ACCESS_KEY=test-secret \
GITHUB_ENV=$r2_env_file "$script_dir/configure-sccache-r2.sh" read >/dev/null
grep -Fx 'SCCACHE_GHA_ENABLED=false' "$r2_env_file" >/dev/null
grep -Fx 'SCCACHE_S3_RW_MODE=READ_ONLY' "$r2_env_file" >/dev/null
grep -Fx 'AWS_ACCESS_KEY_ID=test-read' "$r2_env_file" >/dev/null
if grep -Fx 'SCCACHE_S3_RW_MODE=READ_WRITE' "$r2_env_file" >/dev/null; then
    printf 'read-only R2 configuration unexpectedly allowed writes\n' >&2
    exit 1
fi
if R2_BUCKET=colossus-sccache \
    R2_ENDPOINT=https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com/colossus-sccache \
    R2_REGION=auto R2_ACCESS_KEY_ID=test-write R2_SECRET_ACCESS_KEY=test-secret \
    GITHUB_ENV=$r2_env_file "$script_dir/configure-sccache-r2.sh" write >/dev/null 2>&1; then
    printf 'R2 endpoint with a bucket path unexpectedly succeeded\n' >&2
    exit 1
fi
R2_BUCKET=colossus-sccache \
R2_ENDPOINT=https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com \
R2_REGION=auto R2_ACCESS_KEY_ID=test-write R2_SECRET_ACCESS_KEY=test-secret \
GITHUB_ENV=$r2_env_file "$script_dir/configure-sccache-r2.sh" write >/dev/null
grep -Fx 'SCCACHE_S3_RW_MODE=READ_WRITE' "$r2_env_file" >/dev/null
rm -f "$r2_env_file"
trap - EXIT HUP INT TERM

"${NODE:-node}" --test "$script_dir/cargo-sccache.test.mjs"
"${NODE:-node}" --test "$script_dir/sdk-release.test.mjs"
"${NODE:-node}" --test "$script_dir/homebrew-formula.test.mjs"
"${NODE:-node}" --test "$script_dir/ripgrep-pin.test.mjs"
"${NODE:-node}" --test "$script_dir/release-oci.test.mjs"
"${NODE:-node}" --test "$script_dir/control-plane-assets.test.mjs"
"${NODE:-node}" --test "$script_dir/control-plane-chart.test.mjs"
"${NODE:-node}" --test "$script_dir/documentation-redirects.test.mjs"
"${NODE:-node}" --test "$script_dir/documentation-candidate.test.mjs"
"${NODE:-node}" --test "$script_dir/release-source-version.test.mjs"
"${NODE:-node}" --test "$script_dir/release-readiness.test.mjs"
"${NODE:-node}" --test "$script_dir/browser-pki-evidence.test.mjs"
"${NODE:-node}" --test "$script_dir/release-notes.test.mjs"
"${NODE:-node}" --test "$script_dir/../development-launch.test.mjs"

# These offline integrity and harness tests use synthetic archives and executable
# fixtures. The selected Python needs cryptography for PKI input contracts; see
# docs/develop/testing.md. Ordinary CI never downloads CEF or launches Chromium.
"${PYTHON:-python3}" -B -m unittest discover \
    -s "$script_dir/../../native/browser/scripts" -p 'test_*.py'
"${PYTHON:-python3}" -B -m unittest discover \
    -s "$script_dir/../../native/browser/tls-broker/scripts" -p 'test_*.py'

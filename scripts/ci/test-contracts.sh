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
desktop_required=true' crates/colossus-darwin-process/src/lib.rs
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

"${NODE:-node}" --test "$script_dir/sdk-release.test.mjs"
"${NODE:-node}" --test "$script_dir/homebrew-formula.test.mjs"
"${NODE:-node}" --test "$script_dir/ripgrep-pin.test.mjs"
"${NODE:-node}" --test "$script_dir/release-oci.test.mjs"
"${NODE:-node}" --test "$script_dir/release-source-version.test.mjs"

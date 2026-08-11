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
desktop_required=true' apps/desktop/src/App.tsx scripts/desktop-dev scripts/package-desktop-macos scripts/patch-desktop-manifest-binding.mjs scripts/prepare-desktop-binaries scripts/write-desktop-bundle-manifest.mjs scripts/verify-desktop-bundle.mjs scripts/verify-desktop-unsigned-archive.mjs crates/colossus-sdk/src/lib.rs crates/colossus-sidecar/src/main.rs crates/colossus-sidecar-protocol/src/lib.rs
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

node --test "$script_dir/sdk-release.test.mjs"
node --test "$script_dir/homebrew-formula.test.mjs"

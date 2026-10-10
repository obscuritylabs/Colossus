#!/bin/sh

set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repository_root"

resolve_cargo_tool() {
    tool=$1
    if command -v "$tool" >/dev/null 2>&1; then
        command -v "$tool"
        return
    fi

    cargo_home=${CARGO_HOME:-${HOME:?HOME must be set when CARGO_HOME is absent}/.cargo}
    candidate="$cargo_home/bin/$tool"
    if [ -x "$candidate" ]; then
        printf '%s\n' "$candidate"
        return
    fi

    printf 'missing required tool %s; install the pinned version documented in docs/develop/releasing.md\n' "$tool" >&2
    exit 1
}

require_version() {
    tool=$1
    expected=$2
    actual=$($tool --version)
    if [ "$actual" != "$expected" ]; then
        printf 'expected %s, found %s\n' "$expected" "$actual" >&2
        exit 1
    fi
}

run() {
    printf '+ '
    printf '%s ' "$@"
    printf '\n'
    "$@"
}

cargo_deny=$(resolve_cargo_tool cargo-deny)
cargo_audit=$(resolve_cargo_tool cargo-audit)

require_version "$cargo_deny" "cargo-deny 0.20.2"
require_version "$cargo_audit" "cargo-audit 0.22.2"
PATH=$(dirname "$cargo_deny"):$(dirname "$cargo_audit"):$PATH
export PATH

case $(rustc --version) in
    "rustc 1.96.0 "*) ;;
    *)
        printf 'Rust 1.96.0 is required; found %s\n' "$(rustc --version)" >&2
        exit 1
        ;;
esac

legacy_python_sources=$(git ls-files -- '*.py' ':(exclude)sdk/python/**' \
    ':(exclude)deploy/documentation/build-config.py' \
    ':(exclude)scripts/ci/normalize_python_sdist.py' \
    ':(exclude)native/browser/scripts/component.py' \
    ':(exclude)native/browser/scripts/test_component.py' \
    ':(exclude)native/browser/scripts/run_probe.py' \
    ':(exclude)native/browser/scripts/test_probe.py' \
    ':(exclude)native/browser/scripts/stage_macos.py' \
    ':(exclude)native/browser/scripts/test_stage_macos.py' \
    ':(exclude)native/browser/scripts/test_launcher_macos.py' \
    ':(exclude)bundled-plugins/colossus/skills/security-review/scripts/init_review.py' \
    ':(exclude)scripts/tests/test_security_review_workspace.py' \
    ':(exclude)examples/sdk/integration/server.py' \
    ':(exclude)examples/sdk/provider-failure/server.py')
if [ -e pyproject.toml ] || [ -n "$legacy_python_sources" ]; then
    printf 'the active Rust tree must not contain the retired root Python package or tracked Python source outside the maintained public Python SDK and SDK fixtures or approved build/plugin utilities and their tests\n' >&2
    exit 1
fi

run cargo fmt --all -- --check
run cargo clippy --locked --workspace --all-targets -- -D warnings
run cargo test --locked --workspace
run cargo check --locked --manifest-path fuzz/Cargo.toml --all-targets

run cargo deny --locked check -A license-not-encountered licenses sources bans
run cargo deny --locked check -D warnings advisories
# Tantivy 0.26.1 uses `usize` lru keys, so the panicking key destructor required
# by RUSTSEC-2026-0253 is unreachable. Tantivy merged lru 0.18.2 for its next
# registry release in quickwit-oss/tantivy#3034. Remove this exact exception
# with that upgrade. cargo-deny does not report this informational advisory, so
# its advisory policy remains unmodified.
# OpenID Connect verifies public signatures only; no RSA private-key operations.
# Keep this exact exception aligned with deny.toml and ADR 0006.
run cargo audit -D warnings --ignore RUSTSEC-2026-0253 --ignore RUSTSEC-2023-0071 --file Cargo.lock

run cargo deny --manifest-path fuzz/Cargo.toml --config deny.toml --locked check -A license-not-encountered licenses sources bans
run cargo deny --manifest-path fuzz/Cargo.toml --config deny.toml --locked check -D warnings -A advisory-not-detected advisories
run cargo audit -D warnings --file fuzz/Cargo.lock

printf 'local release-readiness verification passed\n'

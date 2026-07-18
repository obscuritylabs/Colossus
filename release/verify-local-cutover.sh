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

    printf 'missing required tool %s; install the pinned version documented in internal/documentation/release-process.md\n' "$tool" >&2
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

if [ -e pyproject.toml ] || [ -n "$(git ls-files '*.py')" ]; then
    printf 'the active cutover tree must not contain the Python package or tracked Python source\n' >&2
    exit 1
fi

run cargo fmt --all -- --check
run cargo clippy --locked --workspace --all-targets -- -D warnings
run cargo test --locked --workspace
run cargo check --locked --manifest-path fuzz/Cargo.toml --all-targets

run cargo deny --locked check -A license-not-encountered licenses sources bans
run cargo deny --locked check -D warnings advisories
run cargo audit -D warnings --file Cargo.lock

run cargo deny --manifest-path fuzz/Cargo.toml --config deny.toml --locked check -A license-not-encountered licenses sources bans
run cargo deny --manifest-path fuzz/Cargo.toml --config deny.toml --locked check -D warnings advisories
run cargo audit -D warnings --file fuzz/Cargo.lock

printf 'local Rust cutover verification passed\n'

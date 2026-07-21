#!/bin/sh

set -eu

if [ "$#" -eq 0 ]; then
    printf 'change classification requires at least one path\n' >&2
    exit 1
fi

rust_required=false
docs_required=false
dependency_required=false

for changed_path in "$@"; do
    case "$changed_path" in
        docs/* | documentation/* | internal/documentation/* | README.md | CHANGELOG.md | SECURITY.md | AGENTS.md | zensical.toml | scripts/docs-site | scripts/generate-doc-redirects)
            docs_required=true
            ;;
        *)
            rust_required=true
            ;;
    esac

    case "$changed_path" in
        Cargo.toml | Cargo.lock | */Cargo.toml | */Cargo.lock | deny.toml | rust-toolchain | rust-toolchain.toml)
            dependency_required=true
            rust_required=true
            ;;
    esac
done

printf 'rust_required=%s\n' "$rust_required"
printf 'docs_required=%s\n' "$docs_required"
printf 'dependency_required=%s\n' "$dependency_required"

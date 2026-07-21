#!/bin/sh

set -eu

if [ "$#" -eq 0 ]; then
    printf 'change classification requires at least one path\n' >&2
    exit 1
fi

rust_required=false
docs_required=false
dependency_required=false
sdk_required=false
desktop_required=false

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
        package*.json | */package*.json | go.mod | go.sum | */go.mod | */go.sum | pyproject.toml | */pyproject.toml | requirements*.txt | */requirements*.txt)
            dependency_required=true
            rust_required=true
            ;;
    esac

    case "$changed_path" in
        api/* | sdk/*)
            sdk_required=true
            ;;
    esac

    case "$changed_path" in
        apps/desktop/* | scripts/desktop-dev | crates/colossus-sdk/*)
            desktop_required=true
            ;;
    esac

    case "$changed_path" in
        .github/workflows/* | .github/rulesets/* | scripts/ci/* | crates/colossus-cli/tests/ci_contract.rs | crates/colossus-cli/tests/support/*)
            sdk_required=true
            desktop_required=true
            ;;
    esac
done

printf 'rust_required=%s\n' "$rust_required"
printf 'docs_required=%s\n' "$docs_required"
printf 'dependency_required=%s\n' "$dependency_required"
printf 'sdk_required=%s\n' "$sdk_required"
printf 'desktop_required=%s\n' "$desktop_required"

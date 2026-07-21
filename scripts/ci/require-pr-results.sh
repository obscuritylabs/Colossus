#!/bin/sh

set -eu

if [ "$#" -ne 7 ]; then
    printf 'usage: %s CLASSIFY_RESULT RUST_REQUIRED RUST_RESULT DOCS_REQUIRED DOCS_RESULT DEPENDENCY_REQUIRED DEPENDENCY_RESULT\n' "$0" >&2
    exit 2
fi

classify_result=$1
rust_required=$2
rust_result=$3
docs_required=$4
docs_result=$5
dependency_required=$6
dependency_result=$7

if [ "$classify_result" != success ]; then
    printf 'change classification did not succeed: %s\n' "$classify_result" >&2
    exit 1
fi

require_selected_result() {
    name=$1
    selected=$2
    result=$3
    case "$selected:$result" in
        true:success | false:skipped)
            return
            ;;
        *)
            printf '%s selection/result mismatch: selected=%s result=%s\n' "$name" "$selected" "$result" >&2
            exit 1
            ;;
    esac
}

require_selected_result rust "$rust_required" "$rust_result"
require_selected_result documentation "$docs_required" "$docs_result"
require_selected_result dependency-policy "$dependency_required" "$dependency_result"

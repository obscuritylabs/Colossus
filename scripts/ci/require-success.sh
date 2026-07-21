#!/bin/sh

set -eu

if [ "$#" -eq 0 ]; then
    printf 'at least one job result is required\n' >&2
    exit 2
fi

failed=false
for result in "$@"; do
    case "$result" in
        *=success)
            ;;
        *=*)
            printf '%s\n' "$result" >&2
            failed=true
            ;;
        *)
            printf 'malformed job result: %s\n' "$result" >&2
            failed=true
            ;;
    esac
done

if [ "$failed" = true ]; then
    exit 1
fi

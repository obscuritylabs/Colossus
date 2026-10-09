#!/usr/bin/env bash

set -euo pipefail

mode=${1:-}
case "$mode" in
  read|write) ;;
  *) printf 'usage: %s read|write\n' "$0" >&2; exit 2 ;;
esac

required=(R2_BUCKET R2_ENDPOINT R2_REGION R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY)
missing=()
for name in "${required[@]}"; do
  if [[ -z "${!name:-}" ]]; then
    missing+=("$name")
  fi
done
if (( ${#missing[@]} )); then
  if [[ "$mode" == read ]]; then
    printf 'R2 compiler cache unavailable; using read-only GitHub cache (missing %s).\n' "${missing[*]}"
    exit 0
  fi
  printf 'R2 compiler cache is incomplete (missing %s).\n' "${missing[*]}" >&2
  exit 1
fi

# Reject a bucket URL copied from the Cloudflare dashboard: sccache appends the
# bucket to the account endpoint itself. Never print configuration or key values.
if [[ ! "$R2_ENDPOINT" =~ ^https://[[:xdigit:]]{32}\.r2\.cloudflarestorage\.com/?$ ]]; then
  printf 'SCCACHE_ENDPOINT must be the HTTPS R2 account endpoint without a bucket path.\n' >&2
  exit 1
fi
if [[ "$R2_REGION" != auto ]]; then
  printf 'SCCACHE_REGION must be auto for Cloudflare R2.\n' >&2
  exit 1
fi
for name in "${required[@]}"; do
  if [[ "${!name}" == *$'\n'* || "${!name}" == *$'\r'* ]]; then
    printf '%s must contain one line.\n' "$name" >&2
    exit 1
  fi
done
: "${GITHUB_ENV:?GITHUB_ENV must name the GitHub Actions environment file}"

{
  printf 'SCCACHE_GHA_ENABLED=false\n'
  printf 'SCCACHE_BUCKET=%s\n' "$R2_BUCKET"
  printf 'SCCACHE_ENDPOINT=%s\n' "$R2_ENDPOINT"
  printf 'SCCACHE_REGION=auto\n'
  printf 'SCCACHE_S3_USE_SSL=true\n'
  printf 'SCCACHE_S3_KEY_PREFIX=colossus-v1\n'
  if [[ "$mode" == read ]]; then
    printf 'SCCACHE_S3_RW_MODE=READ_ONLY\n'
  else
    printf 'SCCACHE_S3_RW_MODE=READ_WRITE\n'
  fi
  printf 'AWS_ACCESS_KEY_ID=%s\n' "$R2_ACCESS_KEY_ID"
  printf 'AWS_SECRET_ACCESS_KEY=%s\n' "$R2_SECRET_ACCESS_KEY"
} >> "$GITHUB_ENV"

printf 'Cloudflare R2 compiler cache configured for %s access.\n' "$mode"

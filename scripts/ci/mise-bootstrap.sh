#!/bin/sh
# One owner for the mise executable pin used by CI and the devcontainer.
# Checksums are from the pinned release's SHASUMS256.txt (raw binaries).
set -eu

version=2026.9.12
case "${1:-}:${2:-}" in
  Linux:X64) platform=linux-x64; sha256=e79ae57945034903aee8aa2ea66b4c7ca9cd4f4edd5a8a78a589cbae6d0f428a ;;
  Linux:ARM64) platform=linux-arm64; sha256=f344c6961190ed2f68e595ed7cb4f03c36c17812bd608886bec799a3082180ff ;;
  macOS:ARM64) platform=macos-arm64; sha256=f20d7cc555a5b0ee7b8a504acbc2583be1b0848d64115cfbe84119ceb705025b ;;
  Windows:X64) platform=windows-x64; sha256=8bc3a2c46246bb00fc49f467c678cf9ff321f707c533ad707061d0f8c7b639b2 ;;
  *) echo "Unsupported mise platform: ${1:-}:${2:-}" >&2; exit 1 ;;
esac

case "${3:-}" in
  '')
    printf 'version=%s\nsha256=%s\n' "$version" "$sha256"
    ;;
  --install)
    # The container needs only Linux installation; GitHub uses mise-action.
    test "${1:-}" = Linux
    test "$#" -eq 4
    destination=$4
    download=$(mktemp)
    trap 'rm -f "$download"' EXIT HUP INT TERM
    curl --fail --silent --show-error --location \
      --connect-timeout 30 --max-time 300 \
      "https://github.com/jdx/mise/releases/download/v$version/mise-v$version-$platform" \
      --output "$download"
    printf '%s  %s\n' "$sha256" "$download" | sha256sum --check --strict
    install -m 0755 "$download" "$destination"
    ;;
  *) echo 'Expected platform arguments, optionally --install DESTINATION' >&2; exit 1 ;;
esac

#!/bin/sh
set -eu

usage() {
    printf '%s\n' "usage: ./install-apparmor.sh /absolute/path/to/colossus"
}

[ "$#" -eq 1 ] || {
    usage >&2
    exit 2
}
[ "$(id -u)" -eq 0 ] || {
    printf '%s\n' "the AppArmor profile installer must run as root" >&2
    exit 1
}
command -v apparmor_parser >/dev/null 2>&1 || {
    printf '%s\n' "apparmor_parser is required" >&2
    exit 1
}

requested_binary=$1
case "$requested_binary" in
    /*) ;;
    *)
        printf '%s\n' "the Colossus executable path must be absolute" >&2
        exit 2
        ;;
esac
[ -f "$requested_binary" ] && [ ! -L "$requested_binary" ] && [ -x "$requested_binary" ] || {
    printf '%s\n' "the Colossus executable must be an executable regular file, not a link" >&2
    exit 1
}
binary=$(realpath -e -- "$requested_binary")
case "$binary" in
    *[!A-Za-z0-9_./-]*)
        printf '%s\n' "the canonical executable path contains unsupported AppArmor characters" >&2
        exit 1
        ;;
esac

# An exact-path profile is only safe when an unprivileged user cannot replace the
# executable or any directory used to resolve it.
candidate=$binary
while [ "$candidate" != "/" ]; do
    owner=$(stat -c '%u' -- "$candidate")
    mode=$(stat -c '%a' -- "$candidate")
    if [ "$owner" -ne 0 ] || [ $((0$mode & 022)) -ne 0 ]; then
        printf 'refusing a replaceable AppArmor attachment: %s must be root-owned and not group/other writable\n' "$candidate" >&2
        exit 1
    fi
    candidate=$(dirname -- "$candidate")
done

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
template=$script_dir/colossus.apparmor.in
[ -f "$template" ] && [ ! -L "$template" ] || {
    printf '%s\n' "the Colossus AppArmor template is missing or linked" >&2
    exit 1
}

generated=$(mktemp /tmp/colossus-apparmor.XXXXXX)
cleanup() {
    rm -f -- "$generated"
}
trap cleanup EXIT HUP INT TERM

awk -v binary="$binary" '{ gsub("@COLOSSUS_BINARY@", binary); print }' \
    "$template" >"$generated"
chmod 0644 "$generated"

# Load the generated profile before making it persistent. A parser rejection leaves
# the existing on-disk profile untouched.
apparmor_parser -r "$generated"
install -o root -g root -m 0644 "$generated" /etc/apparmor.d/colossus

printf 'installed AppArmor user-namespace grant for %s\n' "$binary"

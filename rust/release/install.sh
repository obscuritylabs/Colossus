#!/bin/sh
set -eu

usage() {
    printf '%s\n' "usage: ./install.sh [--prefix PATH]"
}

prefix=${HOME:?"HOME must be set"}/.local
while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            [ "$#" -ge 2 ] || {
                usage >&2
                exit 2
            }
            prefix=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
done

[ -n "$prefix" ] || {
    printf '%s\n' "install prefix cannot be empty" >&2
    exit 2
}

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
source_binary=$script_dir/colossus
[ -f "$source_binary" ] && [ ! -L "$source_binary" ] && [ -x "$source_binary" ] || {
    printf '%s\n' "package colossus binary is missing, linked, or not executable" >&2
    exit 1
}

bin_dir=$prefix/bin
if [ -L "$bin_dir" ]; then
    printf '%s\n' "refusing to install through a linked bin directory: $bin_dir" >&2
    exit 1
fi
mkdir -p -- "$bin_dir"

temporary=$(mktemp "$bin_dir/.colossus.install.XXXXXX")
cleanup() {
    rm -f -- "$temporary"
}
trap cleanup EXIT HUP INT TERM

cp -- "$source_binary" "$temporary"
chmod 0755 "$temporary"
mv -f -- "$temporary" "$bin_dir/colossus"
trap - EXIT HUP INT TERM

printf '%s\n' "installed $bin_dir/colossus"

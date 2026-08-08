#!/bin/sh
set -eu
set -f

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

usage() {
    printf '%s\n' "usage: ./install.sh [--prefix PATH]"
}

prefix=
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

if [ -z "$prefix" ]; then
    [ -n "${HOME:-}" ] || fail "HOME must be set when --prefix is omitted"
    prefix=$HOME/.local
fi
case "$prefix" in
    /*) ;;
    *) fail "install prefix must be absolute" ;;
esac
if printf '%s' "$prefix" | LC_ALL=C grep -q '[[:cntrl:]]'; then
    fail "install prefix cannot contain control characters"
fi

ensure_no_link_components() {
    checked_path=$1
    remainder=${checked_path#/}
    current=
    old_ifs=$IFS
    IFS=/
    for component in $remainder; do
        IFS=$old_ifs
        [ -n "$component" ] || continue
        current=$current/$component
        if [ -L "$current" ]; then
            fail "refusing to install through linked path component: $current"
        fi
        IFS=/
    done
    IFS=$old_ifs
}

directory_mode() {
    if resolved_mode=$(stat -f '%Lp' -- "$1" 2>/dev/null); then
        printf '%s\n' "$resolved_mode"
    else
        stat -c '%a' -- "$1"
    fi
}

file_owner() {
    if resolved_owner=$(stat -f '%u' -- "$1" 2>/dev/null); then
        printf '%s\n' "$resolved_owner"
    else
        stat -c '%u' -- "$1"
    fi
}

require_private_write_directory() {
    checked_directory=$1
    [ -d "$checked_directory" ] && [ ! -L "$checked_directory" ] ||
        fail "installation directory is missing, linked, or not a directory: $checked_directory"
    [ "$(file_owner "$checked_directory")" = "$(id -u)" ] ||
        fail "installation directory is not owned by the current user: $checked_directory"
    mode=$(directory_mode "$checked_directory") || fail "could not inspect directory permissions"
    case "$mode" in
        *[!0-7]*|'') fail "directory permissions are invalid: $checked_directory" ;;
    esac
    if [ $((0$mode & 022)) -ne 0 ]; then
        fail "installation directory is group- or world-writable: $checked_directory"
    fi
}

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
source_binary=$script_dir/colossus
metadata=$script_dir/install-metadata
[ -f "$source_binary" ] && [ ! -L "$source_binary" ] && [ -x "$source_binary" ] ||
    fail "package colossus binary is missing, linked, or not executable"
[ -f "$metadata" ] && [ ! -L "$metadata" ] ||
    fail "package installation metadata is missing or linked"
[ "$(wc -l < "$metadata" | tr -d ' ')" -eq 6 ] || fail "package metadata must contain six fields"

metadata_value() {
    metadata_key=$1
    sed -n "s/^$metadata_key=//p" "$metadata"
}

schema_version=$(metadata_value schema_version)
version=$(metadata_value version)
target=$(metadata_value target)
channel=$(metadata_value channel)
distribution_origin=$(metadata_value distribution_origin)
installer_kind=$(metadata_value installer_kind)
[ "$schema_version" = 1 ] || fail "package metadata schema is unsupported"
printf '%s\n' "$version" | grep -Eq \
    '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-preview\.[1-9][0-9]*)?$' ||
    fail "package metadata version is invalid"
case "$target" in
    aarch64-apple-darwin|x86_64-apple-darwin|aarch64-unknown-linux-musl|x86_64-unknown-linux-musl) ;;
    *) fail "package metadata target is invalid" ;;
esac
case "$channel" in
    stable)
        case "$version" in
            *-preview.*) fail "package metadata channel and version disagree" ;;
        esac
        ;;
    preview)
        case "$version" in
            *-preview.[1-9]*) ;;
            *) fail "package metadata channel and version disagree" ;;
        esac
        ;;
    *) fail "package metadata channel is invalid" ;;
esac
[ "$distribution_origin" = "https://github.com/obscuritylabs/Colossus/releases" ] ||
    fail "package metadata distribution origin is invalid"
[ "$installer_kind" = direct ] || fail "package metadata installer kind is invalid"

binary_version=$(
    "$source_binary" --version
) || fail "package colossus binary did not report its version"
[ "$binary_version" = "colossus $version" ] || fail "package binary version disagrees with metadata"

bin_dir=$prefix/bin
ensure_no_link_components "$bin_dir"
mkdir -p -- "$bin_dir"
ensure_no_link_components "$bin_dir"
require_private_write_directory "$bin_dir"

if [ -n "${XDG_DATA_HOME:-}" ]; then
    case "$XDG_DATA_HOME" in
        /*) receipt_root=$XDG_DATA_HOME ;;
        *) fail "XDG_DATA_HOME must be absolute" ;;
    esac
else
    [ -n "${HOME:-}" ] || fail "HOME must be set when XDG_DATA_HOME is omitted"
    receipt_root=$HOME/.local/share
fi
receipt_dir=$receipt_root/colossus
if printf '%s' "$receipt_dir" | LC_ALL=C grep -q '[[:cntrl:]]'; then
    fail "installation receipt path cannot contain control characters"
fi
ensure_no_link_components "$receipt_dir"
mkdir -p -- "$receipt_dir"
chmod 0700 "$receipt_dir"
ensure_no_link_components "$receipt_dir"
require_private_write_directory "$receipt_dir"

target_binary=$bin_dir/colossus
receipt=$receipt_dir/install.json
if [ -e "$target_binary" ] || [ -L "$target_binary" ]; then
    [ -f "$target_binary" ] && [ ! -L "$target_binary" ] &&
        [ "$(file_owner "$target_binary")" = "$(id -u)" ] ||
        fail "existing installation is linked, non-regular, or not owned by the current user"
fi
if [ -e "$receipt" ] || [ -L "$receipt" ]; then
    [ -f "$receipt" ] && [ ! -L "$receipt" ] &&
        [ "$(file_owner "$receipt")" = "$(id -u)" ] ||
        fail "existing installation receipt is linked, non-regular, or not owned by the current user"
fi

temporary_binary=$(mktemp "$bin_dir/.colossus.install.XXXXXX")
temporary_receipt=$(mktemp "$receipt_dir/.install.json.XXXXXX")
backup_binary=
had_existing=false
binary_committed=false
receipt_committed=false
cleanup() {
    if [ "$binary_committed" = true ] && [ "$receipt_committed" = false ]; then
        if [ "$had_existing" = true ] && [ -n "$backup_binary" ] && [ -f "$backup_binary" ]; then
            mv -f -- "$backup_binary" "$target_binary" ||
                printf '%s\n' "interrupted install could not restore the previous binary" >&2
            backup_binary=
        else
            rm -f -- "$target_binary"
        fi
        binary_committed=false
    fi
    rm -f -- "$temporary_binary" "$temporary_receipt"
    if [ -n "$backup_binary" ]; then
        rm -f -- "$backup_binary"
    fi
}
trap cleanup EXIT HUP INT TERM

cp -- "$source_binary" "$temporary_binary"
chmod 0755 "$temporary_binary"

json_escape() {
    sed 's/\\/\\\\/g; s/"/\\"/g'
}
escaped_prefix=$(printf '%s' "$prefix" | json_escape)
escaped_binary=$(printf '%s' "$target_binary" | json_escape)
cat > "$temporary_receipt" <<EOF
{
  "schemaVersion": 1,
  "channel": "$channel",
  "version": "$version",
  "target": "$target",
  "prefix": "$escaped_prefix",
  "binaryPath": "$escaped_binary",
  "distributionOrigin": "$distribution_origin",
  "installerKind": "$installer_kind"
}
EOF
chmod 0600 "$temporary_receipt"

if [ -e "$target_binary" ]; then
    had_existing=true
    backup_binary=$(mktemp "$bin_dir/.colossus.backup.XXXXXX")
    cp -p -- "$target_binary" "$backup_binary"
fi

# Keep the two-file ownership handoff non-interruptible. Signals are restored as soon
# as the binary and receipt either both commit or the binary is rolled back.
trap '' HUP INT TERM
mv -f -- "$temporary_binary" "$target_binary"
binary_committed=true
if ! mv -f -- "$temporary_receipt" "$receipt"; then
    if [ "$had_existing" = true ]; then
        mv -f -- "$backup_binary" "$target_binary" ||
            fail "receipt failed and the previous binary could not be restored"
        backup_binary=
    else
        rm -f -- "$target_binary"
    fi
    binary_committed=false
    trap cleanup HUP INT TERM
    fail "installation receipt could not be committed; the binary was rolled back"
fi
receipt_committed=true
trap cleanup HUP INT TERM

if [ -n "$backup_binary" ]; then
    rm -f -- "$backup_binary"
    backup_binary=
fi
trap - EXIT HUP INT TERM

printf '%s\n' "installed $target_binary"
printf '%s\n' "recorded direct installation receipt at $receipt"

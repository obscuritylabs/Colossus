#!/bin/sh
set -eu
set -f

repository=obscuritylabs/Colossus
api_origin=https://api.github.com
release_origin=https://github.com/obscuritylabs/Colossus/releases
maximum_metadata_bytes=1048576
maximum_checksum_bytes=512
maximum_archive_bytes=268435456
maximum_expanded_bytes=268435456

fail() {
    printf 'colossus installer: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
usage: install.sh [OPTIONS]

Install a published Colossus CLI release from obscuritylabs/Colossus.

  --version vX.Y.Z             install one exact published version
  --prefix PATH                installation root (default: $HOME/.local)
  --channel stable|preview     release channel (default: stable)
  --dry-run                    resolve and report without downloading an archive
  --no-modify-path             never modify shell profiles (the default)
  --yes                        intentional noninteractive operation
  -h, --help                   show this help
EOF
}

requested_version=
prefix=
channel=stable
dry_run=false
no_modify_path=false
assume_yes=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version requires vX.Y.Z"
            requested_version=$2
            shift 2
            ;;
        --prefix)
            [ "$#" -ge 2 ] || fail "--prefix requires an absolute path"
            prefix=$2
            shift 2
            ;;
        --channel)
            [ "$#" -ge 2 ] || fail "--channel requires stable or preview"
            channel=$2
            shift 2
            ;;
        --dry-run)
            dry_run=true
            shift
            ;;
        --no-modify-path)
            no_modify_path=true
            shift
            ;;
        --yes)
            assume_yes=true
            shift
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

case "$channel" in
    stable|preview) ;;
    *) fail "--channel must be stable or preview" ;;
esac

if [ -z "$prefix" ]; then
    [ -n "${HOME:-}" ] || fail "HOME must be set when --prefix is omitted"
    prefix=$HOME/.local
fi
case "$prefix" in
    /*) ;;
    *) fail "install prefix must be absolute" ;;
esac

if [ -n "$requested_version" ]; then
    case "$channel:$requested_version" in
        stable:v[0-9]*.[0-9]*.[0-9]*) ;;
        preview:v[0-9]*.[0-9]*.[0-9]*-preview.[1-9]*) ;;
        *) fail "requested version does not match the selected channel" ;;
    esac
    printf '%s\n' "$requested_version" | grep -Eq \
        '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-preview\.[1-9][0-9]*)?$' ||
        fail "version must be vX.Y.Z or vX.Y.Z-preview.N"
fi

for command_name in curl tar sed grep sort uniq cmp head wc mktemp awk; do
    command -v "$command_name" >/dev/null 2>&1 || fail "required command is missing: $command_name"
done

kernel=$(uname -s)
machine=$(uname -m)
case "$kernel:$machine" in
    Darwin:arm64|Darwin:aarch64) target=aarch64-apple-darwin ;;
    Darwin:x86_64|Darwin:amd64) target=x86_64-apple-darwin ;;
    Linux:aarch64|Linux:arm64) target=aarch64-unknown-linux-musl ;;
    Linux:x86_64|Linux:amd64) target=x86_64-unknown-linux-musl ;;
    *) fail "unsupported host: $kernel $machine" ;;
esac

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/colossus-install.XXXXXX") ||
    fail "could not create a private temporary directory"
case "$temporary_root" in
    "${TMPDIR:-/tmp}"/colossus-install.*) ;;
    *) fail "temporary directory resolver returned an unexpected path" ;;
esac
cleanup() {
    rm -rf -- "$temporary_root"
}
trap cleanup EXIT HUP INT TERM

write_json_fields() {
    input_path=$1
    output_path=$2
    if ! awk '
        function flush_field() {
            gsub(/^[[:space:]]+/, "", field)
            gsub(/[[:space:]]+$/, "", field)
            if (field != "") {
                print field
            }
            field = ""
        }
        {
            for (position = 1; position <= length($0); position++) {
                character = substr($0, position, 1)
                if (in_string) {
                    field = field character
                    if (escaped) {
                        escaped = 0
                    } else if (character == "\\") {
                        escaped = 1
                    } else if (character == "\"") {
                        in_string = 0
                    }
                } else if (character == "\"") {
                    in_string = 1
                    field = field character
                } else if (character == "{" || character == "[") {
                    flush_field()
                    depth++
                    delimiter[depth] = character
                } else if (character == "}" || character == "]") {
                    flush_field()
                    expected = character == "}" ? "{" : "["
                    if (depth == 0 || delimiter[depth] != expected) {
                        invalid = 1
                    } else {
                        delete delimiter[depth]
                        depth--
                    }
                } else if (character == ",") {
                    flush_field()
                } else {
                    field = field character
                }
            }
            if (in_string) {
                invalid = 1
            } else {
                field = field " "
            }
        }
        END {
            flush_field()
            if (invalid || in_string || escaped || depth != 0) {
                exit 1
            }
        }
    ' "$input_path" > "$output_path"; then
        fail "release metadata is not valid bounded JSON"
    fi
}

fetch_metadata() {
    metadata_url=$1
    metadata_path=$2
    effective_url=$(
        curl -fsS \
            --noproxy '*' \
            --proto '=https' \
            --proto-redir '=https' \
            --max-redirs 0 \
            --connect-timeout 10 \
            --max-time 30 \
            --max-filesize "$maximum_metadata_bytes" \
            --header 'Accept: application/vnd.github+json' \
            --header 'X-GitHub-Api-Version: 2022-11-28' \
            --user-agent 'colossus-bootstrap-installer/1' \
            --output "$metadata_path" \
            --write-out '%{url_effective}' \
            "$metadata_url"
    ) || fail "release metadata request failed (offline, rate limited, or unavailable)"
    [ "$effective_url" = "$metadata_url" ] || fail "release metadata redirected unexpectedly"
    metadata_size=$(wc -c < "$metadata_path" | tr -d ' ')
    [ "$metadata_size" -le "$maximum_metadata_bytes" ] || fail "release metadata is too large"
    write_json_fields "$metadata_path" "${metadata_path}.fields"
}

metadata_tag() {
    sed -n 's/^[[:space:]]*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' "${1}.fields" |
        sed -n '1p'
}

metadata_boolean() {
    field_name=$1
    metadata_path=$2
    sed -n "s/^[[:space:]]*\"$field_name\"[[:space:]]*:[[:space:]]*\([a-z][a-z]*\)[[:space:]]*$/\1/p" "${metadata_path}.fields" |
        sed -n '1p'
}

validate_release_identity() {
    release_path=$1
    expected_tag=$2
    expected_prerelease=$3
    [ "$(metadata_tag "$release_path")" = "$expected_tag" ] ||
        fail "release metadata tag disagrees with the requested version"
    [ "$(metadata_boolean draft "$release_path")" = false ] ||
        fail "draft releases cannot be installed"
    [ "$(metadata_boolean prerelease "$release_path")" = "$expected_prerelease" ] ||
        fail "release metadata disagrees with the requested channel"
}

release_metadata=$temporary_root/release.json
if [ -n "$requested_version" ]; then
    fetch_metadata "$api_origin/repos/$repository/releases/tags/$requested_version" "$release_metadata"
    if [ "$channel" = stable ]; then
        validate_release_identity "$release_metadata" "$requested_version" false
    else
        validate_release_identity "$release_metadata" "$requested_version" true
    fi
    release_tag=$requested_version
elif [ "$channel" = stable ]; then
    fetch_metadata "$api_origin/repos/$repository/releases/latest" "$release_metadata"
    release_tag=$(metadata_tag "$release_metadata")
    printf '%s\n' "$release_tag" | grep -Eq \
        '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' ||
        fail "latest stable release returned an invalid tag"
    validate_release_identity "$release_metadata" "$release_tag" false
else
    release_list=$temporary_root/releases.json
    candidates=$temporary_root/candidates.txt
    fetch_metadata "$api_origin/repos/$repository/releases?per_page=20" "$release_list"
    sed -n 's/^[[:space:]]*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' \
        "${release_list}.fields" > "$candidates"
    release_tag=
    while IFS= read -r candidate; do
        if printf '%s\n' "$candidate" | grep -Eq \
            '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-preview\.[1-9][0-9]*$'; then
            fetch_metadata "$api_origin/repos/$repository/releases/tags/$candidate" "$release_metadata"
            if [ "$(metadata_tag "$release_metadata")" = "$candidate" ] &&
                [ "$(metadata_boolean draft "$release_metadata")" = false ] &&
                [ "$(metadata_boolean prerelease "$release_metadata")" = true ]; then
                release_tag=$candidate
                break
            fi
        fi
    done < "$candidates"
    [ -n "$release_tag" ] || fail "no published preview release was found in the bounded release window"
    validate_release_identity "$release_metadata" "$release_tag" true
fi

version=${release_tag#v}
archive=colossus-$version-$target.tar.gz
checksum=$archive.sha256
asset_names=$temporary_root/asset-names.txt
sed -n 's/^[[:space:]]*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' \
    "${release_metadata}.fields" > "$asset_names"
grep -Fx "$archive" "$asset_names" >/dev/null || fail "release metadata omits $archive"
grep -Fx "$checksum" "$asset_names" >/dev/null || fail "release metadata omits $checksum"

printf 'Colossus install plan\n  channel: %s\n  version: %s\n  target: %s\n  prefix: %s\n  archive: %s\n' \
    "$channel" "$release_tag" "$target" "$prefix" "$archive"
if [ "$dry_run" = true ]; then
    printf '%s\n' 'dry run complete; no archive was downloaded and no files were changed'
    exit 0
fi

download_asset() {
    asset_url=$1
    asset_path=$2
    maximum_bytes=$3
    effective_url=$(
        curl -fsS \
            --noproxy '*' \
            --proto '=https' \
            --proto-redir '=https' \
            --location \
            --max-redirs 3 \
            --connect-timeout 10 \
            --max-time 300 \
            --max-filesize "$maximum_bytes" \
            --user-agent 'colossus-bootstrap-installer/1' \
            --output "$asset_path" \
            --write-out '%{url_effective}' \
            "$asset_url"
    ) || fail "download failed for $(basename "$asset_path")"
    case "$effective_url" in
        "$asset_url"|https://release-assets.githubusercontent.com/*) ;;
        *) fail "release asset redirected to an unexpected host" ;;
    esac
    asset_size=$(wc -c < "$asset_path" | tr -d ' ')
    [ "$asset_size" -le "$maximum_bytes" ] || fail "download is larger than its fixed limit"
}

asset_base=$release_origin/download/$release_tag
archive_path=$temporary_root/$archive
checksum_path=$temporary_root/$checksum
download_asset "$asset_base/$archive" "$archive_path" "$maximum_archive_bytes"
download_asset "$asset_base/$checksum" "$checksum_path" "$maximum_checksum_bytes"

checksum_line=$(sed -n '1p' "$checksum_path")
[ "$(wc -l < "$checksum_path" | tr -d ' ')" -eq 1 ] || fail "checksum sidecar must contain exactly one line"
# Intentional field splitting checks the exact two-field sidecar grammar. Globbing is
# disabled for the complete script.
# shellcheck disable=SC2086
set -- $checksum_line
[ "$#" -eq 2 ] || fail "checksum sidecar has an invalid shape"
expected_digest=$1
checksum_name=$2
printf '%s\n' "$expected_digest" | grep -Eq '^[0-9a-f]{64}$' || fail "checksum digest is invalid"
[ "$checksum_name" = "$archive" ] || fail "checksum sidecar names an unexpected asset"
if command -v sha256sum >/dev/null 2>&1; then
    actual_digest=$(sha256sum "$archive_path")
    actual_digest=${actual_digest%% *}
elif command -v shasum >/dev/null 2>&1; then
    actual_digest=$(shasum -a 256 "$archive_path")
    actual_digest=${actual_digest%% *}
else
    fail "sha256sum or shasum is required to verify the release"
fi
[ "$actual_digest" = "$expected_digest" ] || fail "archive checksum mismatch"

# A compressed archive below the download limit can still expand to many gigabytes, so
# the expanded stream is measured, never stored, before any listing or extraction writes
# to the filesystem. `head` closes the pipe once the limit is exceeded, so a hostile
# archive is abandoned instead of being decompressed in full.
# Decode diagnostics are suppressed here because the table-of-contents pass below is the
# authoritative report for a malformed archive.
expanded_bytes=$(
    tar -xzOf "$archive_path" 2>/dev/null | head -c "$((maximum_expanded_bytes + 1))" | wc -c
)
expanded_bytes=$(printf '%s' "$expanded_bytes" | tr -d ' ')
[ "$expanded_bytes" -le "$maximum_expanded_bytes" ] ||
    fail "expanded archive is larger than its fixed limit"

package=colossus-$version-$target
entries=$temporary_root/archive-entries.txt
sorted_entries=$temporary_root/archive-entries.sorted
expected_entries=$temporary_root/archive-entries.expected
verbose_entries=$temporary_root/archive-entries.verbose
tar -tzf "$archive_path" > "$entries" || fail "archive table of contents is invalid"
[ "$(wc -c < "$entries" | tr -d ' ')" -le 65536 ] || fail "archive table of contents is too large"
LC_ALL=C sort "$entries" > "$sorted_entries"
[ -z "$(uniq -d "$sorted_entries")" ] || fail "archive contains duplicate paths"
{
    printf '%s\n' \
        "$package/" \
        "$package/LICENSE" \
        "$package/README.md" \
        "$package/colossus" \
        "$package/install-metadata" \
        "$package/install.sh"
    if [ "$kernel" = Linux ]; then
        printf '%s\n' \
            "$package/colossus.apparmor.in" \
            "$package/install-apparmor.sh"
    fi
} | LC_ALL=C sort > "$expected_entries"
cmp "$sorted_entries" "$expected_entries" >/dev/null || fail "archive layout contains missing or unexpected paths"
tar -tvzf "$archive_path" > "$verbose_entries" || fail "archive entry metadata is invalid"
while IFS= read -r entry; do
    case "$entry" in
        d*|-*) ;;
        *) fail "archive contains a link or special file" ;;
    esac
done < "$verbose_entries"

extract_root=$temporary_root/extract
mkdir -m 0700 "$extract_root"
tar --no-same-owner --no-same-permissions -xzf "$archive_path" -C "$extract_root" ||
    fail "archive extraction failed"
package_root=$extract_root/$package
if ! { [ -d "$package_root" ] && [ ! -L "$package_root" ]; }; then
    fail "archive root is unsafe"
fi
for regular_file in LICENSE README.md colossus install-metadata install.sh; do
    if ! { [ -f "$package_root/$regular_file" ] && [ ! -L "$package_root/$regular_file" ]; }; then
        fail "archive member is missing, linked, or not regular: $regular_file"
    fi
done
if ! { [ -x "$package_root/colossus" ] && [ -x "$package_root/install.sh" ]; }; then
    fail "archive executables do not have the required mode"
fi

metadata=$package_root/install-metadata
metadata_value() {
    metadata_key=$1
    sed -n "s/^$metadata_key=//p" "$metadata"
}
[ "$(metadata_value schema_version)" = 1 ] || fail "package metadata schema is unsupported"
[ "$(metadata_value version)" = "$version" ] || fail "package metadata version mismatch"
[ "$(metadata_value target)" = "$target" ] || fail "package metadata target mismatch"
[ "$(metadata_value channel)" = "$channel" ] || fail "package metadata channel mismatch"
[ "$(metadata_value distribution_origin)" = "$release_origin" ] || fail "package metadata origin mismatch"
[ "$(metadata_value installer_kind)" = direct ] || fail "package metadata installer kind mismatch"

binary_version=$(
    "$package_root/colossus" --version
) || fail "downloaded binary did not report its version"
[ "$binary_version" = "colossus $version" ] || fail "downloaded binary version mismatch"

if ! "$package_root/install.sh" --prefix "$prefix"; then
    fail "installation failed; the requested Colossus version was not installed"
fi

case ":${PATH:-}:" in
    *":$prefix/bin:"*) ;;
    *)
        # `$PATH` is intentionally literal operator guidance.
        # shellcheck disable=SC2016
        printf 'Add Colossus to this shell with:\n  export PATH="%s/bin:$PATH"\n' "$prefix"
        ;;
esac

# These flags are intentionally consumed even though this installer never mutates a
# shell profile. Retaining them makes unattended and explicitly no-PATH-modification
# invocations stable without granting implicit profile-write authority.
[ "$no_modify_path" = true ] || [ "$assume_yes" = true ] || :

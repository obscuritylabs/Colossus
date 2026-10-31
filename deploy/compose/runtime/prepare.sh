#!/bin/sh
set -eu
umask 077
copy_material() {
    role=$1 uid=$2
    source=/input/$role target=/output/$role
    test -f "$source/SHA256SUMS"
    (cd "$source" && sha256sum -c SHA256SUMS >/dev/null)
    if test -f "$target/SHA256SUMS"; then
        cmp -s "$source/SHA256SUMS" "$target/SHA256SUMS" || { echo 'Retained credentials differ; restore the original material.' >&2; exit 1; }
        (cd "$target" && sha256sum -c SHA256SUMS >/dev/null)
    else
        test -z "$(ls -A "$target")" || { echo 'Unrecorded material volume; refusing adoption.' >&2; exit 1; }
        cp "$source/"* "$target/"
        chmod 0700 "$target"
        chmod 0600 "$target/"*
        chown -R "$uid:$uid" "$target"
    fi
}
prepare_data() {
    chmod 0700 "/data/$1"
    chown "$2:$2" "/data/$1"
}
case "${1:-}" in
    core)
        copy_material controller 10001
        copy_material database 999
        copy_material browser 10001
        prepare_data controller 10001
        prepare_data database 999
        ;;
    hosted)
        copy_material anchor 10001
        copy_material anchor-database 999
        copy_material anchor-browser 10001
        prepare_data anchor-database 999
        ;;
    *) exit 2 ;;
esac

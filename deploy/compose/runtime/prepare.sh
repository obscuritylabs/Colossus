#!/bin/sh
set -eu
umask 077
copy_material() {
    role=$1 uid=$2
    source=/input/$role target=/output/$role
    test -f "$source/server.pem" || test -f "$source/config.json" || test -f "$source/ca.pem"
    cp "$source/"* "$target/"
    chmod 0700 "$target"
    chmod 0600 "$target/"*
    chown -R "$uid:$uid" "$target"
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
        copy_material bootstrap 10001
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

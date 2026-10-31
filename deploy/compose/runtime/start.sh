#!/bin/sh
set -eu
# Only fixed, purpose-specific files become process environment. Values never
# appear in Compose interpolation, command arguments, or provisioning output.
case "${1:-}" in
    controller)
        COLOSSUS_CONTROL_PLANE_DATABASE=$(cat /run/colossus/database-url)
        COLOSSUS_CONTROL_PLANE_AUTH_KEY=$(cat /run/colossus/auth-key)
        COLOSSUS_CONTROL_PLANE_BOOTSTRAP_PASSWORD=$(cat /run/colossus/admin-password)
        export COLOSSUS_CONTROL_PLANE_DATABASE COLOSSUS_CONTROL_PLANE_AUTH_KEY COLOSSUS_CONTROL_PLANE_BOOTSTRAP_PASSWORD
        if test -f /run/colossus/journal-key; then
            COLOSSUS_CHAT_JOURNAL_KEY=$(cat /run/colossus/journal-key)
            COLOSSUS_CHAT_SIGNING_KEY=$(cat /run/colossus/signing-key)
            COLOSSUS_CHAT_WRAPPING_KEY=$(cat /run/colossus/wrapping-key)
            COLOSSUS_CHAT_ANCHOR_TOKEN=$(cat /run/colossus/anchor-token)
            export COLOSSUS_CHAT_JOURNAL_KEY COLOSSUS_CHAT_SIGNING_KEY COLOSSUS_CHAT_WRAPPING_KEY COLOSSUS_CHAT_ANCHOR_TOKEN
        fi
        exec /usr/local/bin/colossus-control-plane /run/colossus/config.json
        ;;
    anchor)
        COLOSSUS_ANCHOR_DATABASE=$(cat /run/colossus/database-url)
        COLOSSUS_ANCHOR_TOKEN=$(cat /run/colossus/anchor-token)
        COLOSSUS_ANCHOR_STORAGE_KEY=$(cat /run/colossus/storage-key)
        export COLOSSUS_ANCHOR_DATABASE COLOSSUS_ANCHOR_TOKEN COLOSSUS_ANCHOR_STORAGE_KEY
        exec /usr/local/bin/colossus-chat-anchor /run/colossus/config.json
        ;;
    *) exit 2 ;;
esac

---
title: Upgrade, backup, and recovery
description: Preserve Cloud databases and identities, perform controlled upgrades, and recover interrupted work.
audience: operator
type: how-to
icon: lucide/database-backup
---

# Upgrade, backup, and recovery

Back up database state and matching identities before an upgrade. Three API
replicas protect application availability; the bundled single PostgreSQL instance
still needs its own backup and recovery plan.

## Keep these together

| Retain | Why |
| --- | --- |
| PostgreSQL backup and point-in-time recovery | Projects, accounts, sessions, tasks, history, receipts, and sync cursors |
| Connector CA, signing key, and server identity | Existing enrollments and certificate renewal |
| Auth/flow key and OIDC configuration | Shared sign-in state across replicas |
| Hosted encryption, signing, and wrapping keys | Read and verify shared hosted journals and credentials |
| Independent hosted anchor database and key | Detect journal rollback independently from the execution database |
| Independently archived cloud audit heads | Compare retained resource hashes after restore |

Helm-generated identity Secrets and database PVCs survive ordinary uninstall.
Reinstall with the same release name/namespace to reuse them. An incomplete
retained identity fails instead of silently replacing a key. Changing an initialized
database Secret does not rotate the SQL password.

## Upgrade a release

1. Take a recoverable backup and retain the current chart version and image digests.
2. Review the release's compatibility notes and your complete Helm values.
3. Upgrade to the published chart with matching server and documentation images.
4. Verify readiness, sign-in, an enrolled agent, a conversation, and hosted execution
   if enabled.

Use the [installation command](kubernetes.md#install-the-release) with the new
version and your reviewed values. SQL migrations run at startup under an advisory
lock. A checksum mismatch fails startup. Helm rollback does not reverse migrations,
account edits, or credential changes; restore incompatible database state from backup.

Renew the connector server leaf before expiry. Change `tls.renewalRevision` to
reissue a generated leaf under the retained CA and roll pods. Existing/supplied
Secrets need an explicit rollout after changes; processes load identities at startup.
Coordinate auth-key rotation across replicas; in-progress sign-ins must restart.

## Back up the cloud database

Use encrypted platform backups and verified TLS. For a portable logical backup,
configure a protected libpq service/password file to keep credentials out of arguments:

```sh
umask 077
pg_dump --dbname='service=colossus-cloud-backup' --format=custom \
  --schema=colossus_control_plane --file=/secure/backups/colossus-cloud.dump
pg_restore --list /secure/backups/colossus-cloud.dump
```

Encrypt and restrict the exported dump. Runtime journals and connector vaults have
their own backup policies.

For an exact audit checkpoint, first pause HPA and any GitOps reconciler so they
cannot restart writers. Stop all API/executor replicas, take the dump, and export
the latest audit heads to an independently protected archive:

```sh
kubectl -n colossus scale deployment/colossus-control-plane --replicas=0
pg_dump --dbname='service=colossus-cloud-backup' --format=custom \
  --schema=colossus_control_plane --file=/secure/backups/colossus-cloud.dump
psql --dbname='service=colossus-cloud-backup' --set=ON_ERROR_STOP=1 \
  --command='COPY (SELECT DISTINCT ON (project_id,entity_kind,parent_id,id) project_id,entity_kind,parent_id,id,revision,chain_hash FROM colossus_control_plane.cloud_audit ORDER BY project_id,entity_kind,parent_id,id,revision DESC) TO STDOUT WITH CSV HEADER' \
  > /secure/backups/colossus-cloud-audit-heads.csv
kubectl -n colossus scale deployment/colossus-control-plane --replicas=3
kubectl -n colossus rollout status deployment/colossus-control-plane
```

Restore your previous replica and autoscaling/reconciler settings afterward. Work
already accepted by independently enrolled runtimes continues; hosted execution
pauses with its API/executor pods. Resource hashes alone cannot detect a database
and its hashes being replaced together, which is why the checkpoint stays separate.

## Restore and verify

Restore into an empty, isolated database owned by the cloud role:

```sh
pg_restore --dbname='service=colossus-cloud-restore' --single-transaction \
  --exit-on-error --no-owner /secure/backups/colossus-cloud.dump
```

Compare migration checksums, audit heads, enrollments/revocations, task/run IDs,
receipts, and sync cursors with the archived checkpoint. For cutover, stop all
replicas, update database references, restore matching secrets, then restart.
Reconcile certificate changes and commands newer than the backup against original
runtime outcomes before resubmitting work. Hosted restore must also satisfy the
independently retained anchors; never roll them back to hide an integrity failure.

## Diagnose an interruption

| Symptom | First check |
| --- | --- |
| Pending pods | Eligible zones, resource requests, and persistent storage |
| Failed readiness | Database URL/role, verified TLS, and migration checksums |
| Failed sign-in | Browser origin, OIDC issuer/client, shared flow key |
| Offline connected agent | Connector process, exact enrollment, certificate, and TLS passthrough |
| Unknown task outcome | Original runtime receipt and local run; do not automatically retry |
| Hosted execution unavailable | External keys, anchor reachability/trust, and ownership/capacity |

Archive retains conversations; it does not delete messages or runs. Maintenance
cleans expired sign-in state and delivered notification metadata, not conversation
history. Do not prune events, cursors, receipts, audit heads, or pending outbox rows
independently: they jointly support replay and recovery.

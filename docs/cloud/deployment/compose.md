---
title: Install with Docker Compose
description: Run a persistent single-host Colossus installation with Hosted Chat enabled by default.
audience: operator
type: how-to
icon: lucide/server
---

# Install with Docker Compose

Compose starts the Control Plane and web app, documentation, verified-TLS PostgreSQL,
and Hosted Chat on one Docker host. Chat includes a separate checkpoint service and
database. Use this installation for evaluation and single-host operation; use
[Helm](kubernetes.md) for multi-node availability and an independently administered
Hosted checkpoint authority.

## Start the installation

Have a Linux Docker engine with Compose 2.24+ and PowerShell 7.2+ available. On Windows
or macOS, use Docker Desktop's Linux engine. Node 22+ is needed for the acceptance
command. No Kubernetes, Helm, local Rust build, or external identity provider is needed.

Download the release's Control Plane server bundle, verify its adjacent checksum,
and extract it. From the extracted directory:

```powershell
pwsh ./deploy/compose/compose.ps1 -Action up -Name local
```

The helper selects matching published server and documentation images and records
their registry digests. PostgreSQL and the HTTPS proxy also use pinned images.
The first startup generates private credentials and certificate authorities. It
prints file paths for the initial administrator login and browser trust certificate.
It does not print credentials or change your operating system's trust store.

Open `https://localhost:9443`. Review and trust the generated `browser-ca.pem` in your
test browser before signing in as `admin` with the password from `admin-login.txt`.
The **Local Colossus** project has Chat ready with the offline Echo provider. Echo
makes no vendor request. Configure a real provider through the normal connection
settings when you are ready to use a model.

Chat is enabled by default. For an installation that only manages independently
enrolled Fleet runtimes, pass `-CoreOnly` on the first startup. That mode does not
allocate the Hosted anchor or its database.

## Retain state and verify the installation

The private allocation is `~/.colossus-deployments/compose-NAME`. It contains the
reviewed Compose configuration, deployment ownership receipt, generated credentials,
certificates and keys. Persistent Docker volumes contain the databases and each
service's material. The helper validates ownership and retained material before
operating. Keep both the allocation and its recorded Docker engine.

```powershell
pwsh ./deploy/compose/compose.ps1 -Action status -Name local
pwsh ./deploy/compose/compose.ps1 -Action test -Name local
pwsh ./deploy/compose/compose.ps1 -Action down -Name local
pwsh ./deploy/compose/compose.ps1 -Action up -Name local
```

`test` verifies HTTPS, database readiness, anonymous-access rejection, administrator
login, documentation/search and connector HTTP/2 with required client certificates.
For the default Chat installation it also runs one offline Echo conversation through
the HTTPS/SSE route. Later tests reuse that saved conversation, including after
restart or restore. The test refuses to run a model turn if the `main` provider has
been changed from Echo. It never substitutes a vendor credential.

`down` removes containers and networks while retaining volumes and credentials.
Missing retained volumes or keys stop startup. Restore the original material rather
than initializing a replacement. Setup never resets administrator edits or imports
your Desktop/CLI credentials.

## Back up, restore and upgrade

Keep private backups outside the deployment allocation. A backup briefly stops the
Control Plane and anchor, dumps both databases, copies retained configuration and
keys, and resumes the services. The ZIP and adjacent checksum are written to a
protected directory. The archive contains secrets; protect and encrypt transported
copies according to your operating procedures.

```powershell
pwsh ./deploy/compose/compose.ps1 -Action backup -Name local
```

The command prints the backup path under `~/.colossus-deployments/backups/`. To restore,
retain the ZIP and its `.sha256` sidecar. Stop the old installation so its browser and
connector ports are available, then restore into a fresh name:

```powershell
pwsh ./deploy/compose/compose.ps1 -Action down -Name local
pwsh ./deploy/compose/compose.ps1 -Action restore -Name recovered -Backup /absolute/private/backups/local.zip
pwsh ./deploy/compose/compose.ps1 -Action test -Name recovered
```

Restore preserves the original hostname, ports, account identities, history and keys,
but allocates new Docker resources and an ownership receipt for the new installation.
The original volumes are retained. Protected `restore-input` evidence stays in the
new allocation until you have verified recovery.

Upgrade to an explicitly selected published release:

```powershell
pwsh ./deploy/compose/compose.ps1 -Action upgrade -Name local -ReleaseTag vX.Y.Z
```

Upgrade first makes a quiesced backup, then replaces the selected server and
documentation images while retaining state and keys. It does not automatically
downgrade after a failed migration. Restore the retained backup into a fresh
installation if rollback is required. This command updates images; review deployment
helper/configuration changes in the new release bundle before adopting them.

## Bind another host or remove an installation

The default published address is loopback. For a reachable server, choose the real
DNS hostname and explicit bind address during first setup, with unused browser and
connector ports:

```powershell
pwsh ./deploy/compose/compose.ps1 -Action up -Name lab -HostName colossus.example.com -BindAddress 0.0.0.0 -Port 9443 -ConnectorPort 9444
```

Provide DNS and firewall rules for those ports. Setup issues certificates for the
selected hostname; clients must trust the corresponding generated authority. Browser
HTTPS and connector mTLS use different authorities. Generated leaves are valid for
one year; renewal requires a deliberate material update and is not automatic in this
version. Never terminate the connector's client-certificate session in an HTTP proxy.
Expired certificates block startup; backup and shutdown remain available.
See [Connect runtimes](../connect-runtimes.md) for enrollment; select this installation's
`browser-ca.pem` with `--enrollment-ca-file`.

Preview permanent removal of this exact installation's database volumes:

```powershell
pwsh ./deploy/compose/compose.ps1 -Action down -Name local -RemoveData -WhatIf
```

Remove `-WhatIf` only when you intend to discard the databases. Docker resource labels
must match the retained deployment identity; the helper refuses foreign resources.
It does not prune shared Docker images or remove other installations. It retains
the private allocation after data removal for recovery inspection.

## Understand the Hosted boundary

The anchor has a separate database, role, encryption key and private network. The
Control Plane does not receive the anchor database password or storage key. However,
both services share one Docker host and administrator. This arrangement exercises
the Hosted storage protocol; it does not establish independent rollback protection
against that administrator. For that boundary, follow the [Hosted production guide](hosted-chat.md).

Compose supplies no native execution environment or Docker socket to agents. Hosted
execution retains the existing controlled tool ceiling. Filesystem, native shell,
browser and computer operations use separately enrolled Fleet workers and their
existing permissions.

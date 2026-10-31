---
title: Install with Docker Compose
description: Start single-host Colossus with Chat enabled, then operate it with ordinary Docker Compose commands.
audience: operator
type: how-to
icon: lucide/server
---

# Install with Docker Compose

Compose runs the Control Plane, web app, documentation, PostgreSQL and Hosted Chat
on one Docker host. Use [Helm](kubernetes.md) for multi-node availability and an
independently administered production checkpoint authority.

## Initialize once

Install a Linux Docker engine with Compose 2.24+ and PowerShell 7.2+. Docker
Desktop's Linux engine works on Windows and macOS. Download the release's Control
Plane server bundle, verify its adjacent checksum and extract it. From the bundle:

```powershell
cd deploy/compose
pwsh ./init.ps1
docker compose up -d --wait
```

Initialization selects the accompanying release version and writes non-secret
Compose settings to `.env`. Private credentials, certificates and service
configuration go into `.local/`; existing output is never replaced. The checked-in
`.env.example` and `config/` examples describe the generated configuration. No
GitHub account, Actions workflow, Kubernetes or local Rust build is required.

Open `https://localhost:9443`. Review and trust `.local/material/browser/ca.pem`
in your test browser, then sign in as `admin` using `.local/admin-login.txt`.
The one-shot setup container creates **Local Colossus**, an explicit operator
membership and Chat with the offline Echo provider. Later restarts skip bootstrap
and preserve administrator edits. Configure real providers through the normal
connection settings; Echo makes no vendor request.

## Operate with Compose

```sh
docker compose ps
docker compose logs -f
docker compose down
docker compose up -d --wait
```

`down` retains named volumes. Keep those volumes and `.local/` together: they hold
the databases, checkpoint state, credentials and encryption keys. Back up both
PostgreSQL databases and private material before explicitly updating the image
references in `.env`. This adapter adds no upgrade, backup or restore framework.
See [operations and recovery](operations.md) for the service-level responsibilities.

`docker compose down --volumes` permanently deletes this Compose project's data.
Do not use it for a normal shutdown. Setup files remain private on disk afterward.

## Choose addresses and understand the boundary

The default published address is loopback. During first initialization, select a
different DNS hostname and ports with `-HostName`, `-Port` and `-ConnectorPort`.
For remote access, explicitly change `COLOSSUS_BIND_ADDRESS` in `.env` and provide
DNS and firewall rules. Generated certificates include the selected hostname.
Browser HTTPS and connector mTLS use different authorities; never terminate the
connector's client-certificate session in an HTTP proxy. For enrollment, use
`.local/material/browser/ca.pem` with `--enrollment-ca-file`, following
[Connect runtimes](../connect-runtimes.md).

Generated leaves last one year; renewal requires deliberate material replacement
and service recreation. Initialization does not rotate keys or modify system trust.
Database transport remains verified TLS. Hosted Chat requires its separate HTTPS
checkpoint service and database for this Docker bridge topology.

The checkpoint service has separate credentials, encryption keys and networking,
but shares the Docker administrator. It does not provide independent rollback
protection against that administrator. Follow the
[Hosted production guide](hosted-chat.md) when that boundary is required.
Compose supplies no agent Docker socket or native execution environment. Native
CLI/TUI and Fleet workers remain separately installed and enrolled.

The **Compose smoke tests** GitHub workflow starts and removes temporary test
containers only. It is not a deployment service and is not required to operate
your installation.

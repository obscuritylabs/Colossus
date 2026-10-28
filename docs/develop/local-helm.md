---
title: Test Helm locally on Windows
description: Run an isolated Control Plane in Docker, verify it, and remove its test resources.
audience: developer
type: how-to
---

# Test Helm locally on Windows

Use the local harness to test the chart in a Docker-backed kind cluster. It creates
one server, one documentation pod, and a persistent PostgreSQL database. Envoy Gateway
terminates browser HTTPS and passes the connector's mutual TLS through unchanged.
This single-node profile exercises deployment; it does not establish production
availability-zone, capacity, or NetworkPolicy enforcement.

## Prepare the tools

Have Docker Desktop running its Linux engine, with hardware virtualization enabled.
Install PowerShell 7, Git, kind, Helm 3.19+, kubectl, and Node.js from the
[source toolchain](toolchain-inventory.md). Put their executables on `PATH`, or pass
`-Kind`, `-Helm`, `-Docker`, `-Kubectl`, and `-Node` paths to the harness. Node runs the
connector TLS check; without it, the receipt records that check as skipped.

Run from a source checkout containing the chart version you want to test. The default
server and documentation images match that version and must already be published.
The harness downloads a pinned Envoy Gateway chart and uses a digest-pinned Kubernetes
node image. Initial setup requires network access and enough Docker memory and disk
for the cluster and three application images.

## Start a disposable deployment

Use a distinct name for each deployment. This example uses port 8443 so it can run
beside an existing local installation on 443:

```powershell
pwsh ./scripts/helm-local.ps1 -Action up -Name sandbox -Port 8443
```

Setup waits for the cluster and applications, then checks health, anonymous access
denial, administrator sign-in, the web entry point, documentation search, and connector
TLS. A failed command retains its files for inspection. Run the same command again
to retry an owned deployment; changing its port requires a new name or cleanup first.

Open `https://colossus.localhost:8443`. Read the generated `admin` credential privately
from `.local/tests/helm-sandbox/private/admin-login.txt`. The certificate is self-signed;
the harness does not change Windows trust, browser profiles, or the hosts file.
Configure your test browser's trust deliberately before signing in. Command-line
smoke checks trust the exact generated certificate and resolve the hostname to loopback.

The advertised connector endpoint is `https://runtimes.colossus.localhost:8443`.
The native connector resolves the reserved `localhost` domain and its subdomains
to loopback, including on Windows. Select the generated `browser-cert.pem` beside
the printed private `admin-login.txt` for local CLI enrollment:

```powershell
colossus cloud enroll https://colossus.localhost:8443/api/enroll `
  --enrollment-ca-file C:/secure/browser-cert.pem
```

Use the actual certificate path from your deployment; it is the public trust anchor,
not its private key. The CLI verifies the original hostname and certificate. The
connector's subsequent mTLS connection uses the independently enrolled connector CA.
Browser certificate exceptions and Windows trust-store imports do not select CLI trust.
Follow [Connect runtimes](../cloud/connect-runtimes.md), then use the
[web shell procedure](../cloud/work.md#open-a-shell-on-a-connected-host).
Terminal access still requires the worker opt-in, connector grant, and project role.

## Check or upgrade the same installation

```powershell
pwsh ./scripts/helm-local.ps1 -Action test -Name sandbox
pwsh ./scripts/helm-local.ps1 -Action up -Name sandbox -Port 8443
```

`test` reads the recorded port and uses the dedicated kubeconfig. `up` performs a Helm
upgrade with the current checkout's chart and selected images. Keep the same deployment
name to retain its database, auth key, and connector identity. Back up anything valuable
before testing an upgrade; a Helm rollback does not reverse database migrations.

To test images built from changed source, give both explicit image references and load
them into kind:

```powershell
pwsh ./scripts/helm-local.ps1 -Action up -Name sandbox -Port 8443 `
  -ControlPlaneImage colossus-control-plane:my-change `
  -DocumentationImage colossus-documentation:my-change -LoadImages
```

Build both images first using the repository Dockerfiles. `-LoadImages` selects the
local loaded images without pulling replacements. Record the source commit and image
IDs with your test results; testing a released image does not verify unbuilt source.

## Find the evidence and clean up

All harness state belongs under the **primary checkout's** `.local/tests/helm-NAME`,
including when the script runs from a linked worktree. The directory holds the ownership
manifest, private kubeconfig and certificates, client caches, logs, and verification
receipts. The process restores its original client environment when it exits. Docker
retains the kind node and its database in the Linux engine until cleanup.

Preview cleanup, then remove the exact owned cluster:

```powershell
pwsh ./scripts/helm-local.ps1 -Action down -Name sandbox -WhatIf
pwsh ./scripts/helm-local.ps1 -Action down -Name sandbox
```

Deleting this cluster deletes its PostgreSQL data. Files remain by default so you can
inspect failures and retain receipts. After exporting anything needed, remove the owned
run directory explicitly:

```powershell
pwsh ./scripts/helm-local.ps1 -Action down -Name sandbox -RemoveFiles
```

Cleanup validates the exact manifest, resource identities, path boundary, and absence
of reparse points. It refuses unknown directories and deployments created by older
harnesses. `-RemoveImages` removes only images recorded as owned and whose IDs and
ownership labels still match. It does not remove supplied images merely because you
loaded them into the test cluster. `-WhatIf` makes no Docker or kind calls. No action
performs a global Docker prune or searches your user profile for folders to delete.

For production prerequisites, backups, and routing choices, use
[Install with Helm](../cloud/deployment/kubernetes.md) and
[deployment operations](../cloud/deployment/operations.md).

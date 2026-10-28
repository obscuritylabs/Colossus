---
title: Test Helm locally on Windows
description: Run the complete local Control Plane in Docker, verify it, and remove its owned test resources.
audience: developer
type: how-to
---

# Test Helm locally on Windows

The local harness installs the web UI, documentation, Fleet, and Hosted execution in
a Docker-backed kind cluster. Hosted state uses verified-TLS PostgreSQL and an HTTPS
checkpoint authority with a separate encrypted database. The production chart keeps
Hosted opt-in; the local harness supplies its keys, databases, and configuration.

This is one local node. Three API replicas exercise cross-replica behavior; they do
not provide availability-zone resilience or establish NetworkPolicy enforcement.

## Prepare the tools

Have Docker Desktop running its Linux engine, with hardware virtualization enabled.
Allocate at least 16 GiB of Docker memory for this profile. It runs three API pods,
one documentation pod, two PostgreSQL instances, the checkpoint authority, and Envoy.
Each API pod permits two resident Hosted runtimes and a 1 GiB disposable cache.

Install PowerShell 7.2 or newer, Git, kind, Helm 3.19+, kubectl, and Node.js from the
[source toolchain](toolchain-inventory.md). Put their executables on `PATH`, or pass
`-Kind`, `-Helm`, `-Docker`, `-Kubectl`, and `-Node` paths to the harness. Node is required
for the Hosted API and connector checks. Certificates and keys use PowerShell/.NET;
Python and a separate PKI tool are unnecessary.

Run from a source checkout containing the chart version you want to test. The default
server and documentation images match that version and must already be published.
The harness uses a pinned Envoy Gateway package and digest-pinned dependency images.
Initial setup needs network access and enough disk for those images, an 8 GiB
controller database volume, and a 1 GiB checkpoint-authority database volume.

## Start a disposable deployment

Choose a fresh name and an unused port. This example leaves installations on ports 443 and
8443 available:

```powershell
pwsh ./scripts/helm-local.ps1 -Action up -Name sandbox -Port 9443
```

Setup creates the owned resources and waits for readiness. It checks health,
administrator sign-in, Hosted administration APIs, the web entry point,
documentation search, and connector TLS. A failed command retains its files and
ownership receipts for inspection. Reuse the same name to resume an owned current
profile; changing the port requires another name or cleanup first. Only one
`up`, `test`, or `down` operation can run per deployment. If it reports busy, let
the current operation finish and inspect its result before trying again.

The project **Local Colossus** (`local`) starts with the offline Echo provider and
the supported controlled tool configuration. The initial administrator receives the
Terminal Operator role in that project so it can execute Hosted work. No vendor
credential is imported and no paid provider call is needed for setup. Configure a
real provider deliberately in the web UI before using it for model work.

## Sign in and connect runtimes

Open `https://colossus.localhost:9443`. Read the generated `admin` credential privately
from the `admin-login.txt` path printed by the harness, inside its protected allocation.
The password is not printed. The certificate is self-signed. Configure your test browser's trust
before signing in. The harness does not change Windows trust, browser profiles, or
the hosts file. Its command-line checks trust the exact certificate and resolve the
hostname to loopback.

The web administration pages are available immediately. Use **Local Colossus** to
test Hosted chat and managed files. Hosted execution and an enrolled Fleet worker
are separate targets; starting this cluster does not enroll a personal runtime.

The advertised connector endpoint is `https://runtimes.colossus.localhost:8443`.
The native connector resolves the reserved `localhost` domain and its subdomains
to loopback, including on Windows. Select the generated `browser-cert.pem` beside
the printed private `admin-login.txt` for local CLI enrollment:

```powershell
colossus cloud enroll https://colossus.localhost:9443/api/enroll `
  --enrollment-ca-file C:/secure/browser-cert.pem
```

The advertised connector endpoint is `https://runtimes.colossus.localhost:9443`.
Use the actual certificate path from your deployment; it is the public trust anchor,
not its private key. The CLI verifies the original hostname and certificate. The
connector's subsequent mTLS connection uses the independently enrolled connector CA.
Browser certificate exceptions and Windows trust-store imports do not select CLI trust.
Follow [Connect runtimes](../cloud/connect-runtimes.md), then use the
[web shell procedure](../cloud/work.md#open-a-shell-on-a-connected-host).
Remote shell access still requires the worker opt-in, connector grant, and project role.
The harness does not grant model tools a human terminal.

## Check or upgrade the same installation

```powershell
pwsh ./scripts/helm-local.ps1 -Action test -Name sandbox
pwsh ./scripts/helm-local.ps1 -Action up -Name sandbox -Port 9443
```

`test` uses the recorded port, dedicated kubeconfig, and existing profile. `up` uses
the current checkout's chart and selected images while retaining the deployment's
database identities, auth key, connector identity, Hosted masters, and anchor state.
A missing retained key fails setup; deleting a key is not a recovery procedure.
Back up valuable state before upgrading. Helm rollback does not reverse database
migrations or account changes; see [deployment recovery](../cloud/deployment/operations.md).

To test images built from changed source, give both explicit image references and load
them into kind:

```powershell
pwsh ./scripts/helm-local.ps1 -Action up -Name sandbox -Port 9443 `
  -ControlPlaneImage colossus-control-plane:my-change `
  -DocumentationImage colossus-documentation:my-change -LoadImages
```

Build both images first using the repository Dockerfiles. `-LoadImages` uses the
selected local images without pulling replacements. Record the source commit and
image IDs; released images do not verify unbuilt source changes.

### Keep an older deployment intact

Schema 2 manifests belong to the earlier local profile. Current `up` and `test` reject
them before changing files, client settings, or cluster resources. This also protects
an older installation that was extended manually: the harness cannot infer ownership
of its databases, keys, and external authority from a profile name.

Use a fresh `-Name` to try the complete profile alongside it. A new deployment starts
with new accounts and data; it does not import the old database or enrollments. Keep
the existing deployment and matching keys until a backed-up database migration is
reviewed and verified. `down` and `down -WhatIf` still recognize the older ownership
manifest for deliberate cleanup. Previews also remain available; an older-profile
`up -WhatIf` or `test -WhatIf` reports the migration requirement without changing state.

## Find the evidence and clean up

The **primary checkout's** `.local/tests/helm-NAME` holds the ownership manifest,
client caches, logs, and verification receipts, even from a linked worktree. The
kubeconfig, credentials, PKI, and master material use one protected allocation under
`%USERPROFILE%/.colossus-tests/helm-NAME-UUID`. The manifest binds that exact allocation
to the deployment; repeated `up` reuses it. No action changes the permanent user
environment or scans the profile for similarly named directories.

Preview cleanup, then remove the exact owned cluster:

```powershell
pwsh ./scripts/helm-local.ps1 -Action down -Name sandbox -WhatIf
pwsh ./scripts/helm-local.ps1 -Action down -Name sandbox
```

Deleting the cluster deletes both PostgreSQL databases. Run files and private material
remain by default for inspection and recovery. After exporting what you need, remove
the exact owned run directory and private allocation explicitly:

```powershell
pwsh ./scripts/helm-local.ps1 -Action down -Name sandbox -RemoveFiles
```

If the manifest records locally built owned images, also supply `-RemoveImages` before
discarding their receipts. Image removal requires the recorded ID and ownership labels;
supplied or published images do not become owned merely because they were loaded.

Cleanup checks the manifest, container identities, private allocation, resolved path
boundaries, and reparse points. Unknown or replaced resources stop cleanup. `-WhatIf`
does not create files, generate credentials, or invoke Docker, kind, or Helm.
No action performs a global Docker prune or removes the protected test parent.

For production prerequisites and independently administered services, use
[Install with Helm](../cloud/deployment/kubernetes.md) and
[Hosted deployment](../cloud/deployment/hosted-chat.md).

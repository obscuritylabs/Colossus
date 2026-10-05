---
title: Connect runtimes to Colossus Cloud
description: Deploy the Kubernetes control plane and explicitly enroll CLI or Desktop runtimes.
audience: operator
type: how-to
---

# Connect runtimes to Colossus Cloud

Colossus Cloud starts and controls tasks on explicitly enrolled runtimes. A runtime
opens an outbound connection; its workspace and daemon need no inbound public port.
The cloud host owns project membership and durable placement. The local runtime
continues to own provider credentials, tools, policy, approvals, sandboxing and audit.

## Deploy on Kubernetes

Use the source deployment under `deploy/cloud/kubernetes`. Set the built image in
`kustomization.yaml`, replace both example hostnames, and configure `cloud.json` with
your OIDC issuer, relying-party client, exact subject/project memberships and PostgreSQL
schema. Register the exact browser `/auth/callback` URL with the identity provider.
Sign-in uses authorization code, S256 PKCE, state and nonce; project permissions are
explicit independent `read`, `execute`, `control`, `approve` and `administer` grants.

Create the namespace and supply Secrets through your normal secret manager:

```sh
kubectl create namespace colossus
kubectl -n colossus create secret generic colossus-cloud-state \
  --from-file=database=/secure/cloud/database-url \
  --from-file=journal-key=/secure/cloud/journal-key \
  --from-file=signing-key=/secure/cloud/signing-key
kubectl -n colossus create secret generic colossus-cloud-oidc \
  --from-file=client-secret=/secure/cloud/oidc-client-secret
kubectl -n colossus create secret generic colossus-cloud-tls \
  --from-file=ca.pem=/secure/cloud/ca.pem \
  --from-file=ca-key.pem=/secure/cloud/ca-key.pem \
  --from-file=server.pem=/secure/cloud/server.pem \
  --from-file=server-key.pem=/secure/cloud/server-key.pem
kubectl -n colossus create secret tls colossus-browser-tls \
  --cert=/secure/cloud/browser.pem --key=/secure/cloud/browser-key.pem
kubectl apply -k deploy/cloud/kubernetes
kubectl -n colossus rollout status deployment/colossus-cloud
```

The journal encryption key and Ed25519 signing seed are separate random 32-byte hex
values. Keep their values out of Git and command arguments. Preserve the anchor PVC
with the database, and follow [state recovery](../develop/state-recovery.md) when restoring
both. PostgreSQL verifies TLS with WebPKI roots by default; private database CAs use
the existing adapter's `custom_ca` configuration and a mounted PEM bundle.

This deployment uses one host replica and `Recreate` updates. Session and connection
ownership are process-local; horizontal scaling requires a separate ownership design.
Do not increase replicas or attach an HPA to this deployment.

Browser HTTPS terminates at the trusted ingress. The separate runtime hostname on
port 443 passes TCP/TLS through to port 8443, preserving HTTP/2 and client certificates.
The runtime server leaf must cover that hostname and chain to the configured connector
CA. The checked-in ingress example uses ingress-nginx's
[SSL passthrough](https://kubernetes.github.io/ingress-nginx/user-guide/tls/#ssl-passthrough);
enable its controller flag or use your environment's equivalent TCP forwarding.
Keep browser SSE buffering disabled. `/health/live` reports the host process;
`/health/ready` probes canonical storage. SIGTERM closes streams and drains the host.

## Enroll a CLI daemon

Start an existing configured worker with an owner-private public API directory.
Stop it briefly to enroll a dedicated application using the same directory and
configuration, then restart it. Offline enrollment refuses another active owner.

```sh
colossus --workspace /work/project --config /secure/runtime.yaml worker \
  --public-api-dir /secure/runtime-api \
  --enroll-application app:colossus-cloud-cli \
  --scope runs:execute --scope runs:read --scope runs:control \
  --scope prompts:respond --scope approvals:respond \
  --role primary --tool user.ask --tool filesystem.read \
  --credential-keyring-service dev.colossus.cloud \
  --credential-keyring-account engineering
colossus --workspace /work/project --config /secure/runtime.yaml worker \
  --public-api-dir /secure/runtime-api
```

Choose tools and roles explicitly beneath the runtime's existing ceilings. The native
credential store receives the bearer directly; enrollment output contains public metadata
only. Public API administration is supported on Windows, macOS and Linux with their
native owner-private directory checks.

Create an owner-private `connector.json` containing only public paths, pins and native
credential references. Use the exact values returned by local application enrollment:

```json
{
  "descriptor": "/secure/runtime-api/endpoint.json",
  "certificate": "/secure/runtime-api/certificate.pem",
  "instance_id": "replace-with-enrolled-local-instance",
  "certificate_sha256": "replace-with-exact-64-character-local-pin",
  "keyring_service": "dev.colossus.cloud",
  "keyring_account": "engineering"
}
```

Sign into the web app, select the project, open **Runtime fleet**, and create an
invitation with the reviewed label and role ceiling. On the runtime machine, run:

```sh
colossus cloud enroll --local-config /secure/connector.json \
  --enrollment-url https://colossus.example.com/api/enroll --name engineering
colossus cloud run --local-config /secure/connector.json --name engineering
```

In another terminal, inspect or stop that exact named CLI connection:

```sh
colossus cloud status --name engineering
colossus cloud disconnect --name engineering
colossus cloud revoke --name engineering
```

Status reports a bounded, owner-private, non-secret heartbeat independently of the
encrypted credential vault. A stale running heartbeat is reported as `unknown`.
Disconnect requests are bound to one connector process generation; an old request
cannot stop a later connection. Revocation disconnects first, then uses the native
client certificate to revoke only its own enrolled node. Retry a failed revocation
with the same enrollment; forgetting credentials alone does not revoke cloud authority.

Paste the invitation into the stdin prompt. It expires after ten minutes and is consumed
once. The connector generates and retains its private key locally. Enrollment binds the
cloud node to the independently verified local instance. The separate `colossus-connector`
binary accepts the same commands without the `cloud` prefix.

## Enroll Desktop

Select a Managed Local workspace and open its **Cloud** settings. Paste the web
invitation and enrollment URL, then review the native confirmation showing the cloud
origin and execution/approval authority. Desktop uses a separate native-held application
credential; the renderer receives connection status and public identities only.
External daemon targets use their separately enrolled CLI connector.

Desktop can disconnect, reconnect, revoke, and forget its enrollment. Closing the cloud
connection leaves accepted tasks running locally. Removing a local enrollment does
not revoke it remotely. Use **Revoke enrollment** in Desktop or **Revoke runtime** in
the web fleet to remove cloud authority. Desktop revocation requires native confirmation.

## Install and supervise runtime connections

CLI and standalone connectors use the same outbound protocol on Windows, macOS and
Linux. Use the platform's native credential authority for interactive installations:
Windows Credential Manager, macOS Keychain, or Linux Secret Service. A graphical
credential authority must be unlocked for enrollment, connection and rotation. Use
the explicit sealed headless authority below for unattended hosts that lack one.

Run the daemon and connector as the same selected operating-system user, with distinct
process lifetimes. On Linux, supervise both with systemd; on macOS, use launchd;
on Windows, use your service supervisor or Task Scheduler. Preserve the configured
owner-private runtime and Colossus home directories across restarts. Select the exact
enrollment name and local configuration in the connector's supervised command. Inject
headless wrapping keys through the supervisor's protected secret environment.

For concurrent CLI connectors, give each daemon/connector pair its own owner-private
`COLOSSUS_HOME`. A native vault retains one process's exclusive lease while the
connector is running; named enrollments in one home are selected serially. Desktop
shares its native vault within the application and can connect multiple workspaces.

Use `cloud disconnect` or Ctrl-C for graceful connector shutdown. This leaves the
daemon and accepted runs alive. Stop the worker separately with its normal graceful
shutdown when ending local execution. Revoke before uninstalling a enrolled connector,
then use `cloud forget` to delete its native enrollment. Desktop's embedded connector
and Managed Local runtime end when Desktop exits; persistent execution requires a
separately supervised CLI daemon.

Linux Desktop is distributed as an unsigned Developer Preview `.deb` for Debian 12
compatible systems, in native x86-64 and ARM64 builds. Verify the accompanying SHA-256
checksum and install the package with your distribution's package manager. It includes
sealed sidecar, CLI and search executables and requires GTK 3, WebKitGTK 4.1, Secret
Service and ALSA. A running unlocked Secret Service is required for native credentials.
Launch `colossus-desktop` or the desktop menu entry. Revoke enrolled workspaces before
removing the package. User workspace state remains in the configured Colossus home.

## Use an explicit headless authority

Linux servers without Secret Service can inject an operator-owned random 32-byte hex
wrapping key through a secret manager. Set the worker's
`--public-api-vault-key-variable COLOSSUS_HEADLESS_KEY` for both offline enrollment and
normal hosting. Its public API credentials are sealed beneath the public API directory's
`headless-credentials` child. Add the non-secret selector to `connector.json`:

```json
{
  "headless_authority": {
    "directory": "/secure/runtime-api/headless-credentials",
    "key_variable": "COLOSSUS_HEADLESS_KEY"
  }
}
```

Retain all the earlier connector fields. Supply
`--vault-key-variable COLOSSUS_HEADLESS_KEY` to connector enrollment and lifecycle
commands. The injected key protects the vault's small key envelope; the daemon bearer
and connector TLS private key remain sealed on disk. There is no fallback to plaintext
or another credential store when that authority is missing or changed.

## Verify and recover

The fleet shows a ready node after its authenticated heartbeat. Submit a short task,
open it, and verify live output, pending prompts and approvals, and its terminal state.
Approval requires the independent project `approve` permission plus the native local
grant. Cancellation and ordinary prompt responses require `control`.

Connector certificates last thirty days and renew with a new key at seven days
remaining. `colossus cloud renew --name engineering` explicitly rotates a certificate;
use the same native vault selector for headless deployments. Interrupted enrollment and
renewal retain their exact pending exchange for retry. Expired or revoked enrollment
requires operator reconciliation or a new reviewed invitation.

Cloud and connector restarts replay the same task/run allocation and exclusive durable
output cursor. Disconnection never moves accepted work to another machine. An unknown
outcome stays explicit; inspect the original local run before deciding whether to submit
new work. Never automatically retry it with a fresh task identity.

Each node admits sixteen active tasks. Each task retains at most 4096 output events and
16 MiB of released output. Reaching that bound marks the web task explicitly and switches
to snapshot/control updates; full runtime output remains local. Sessions are bounded by
the verified ID-token expiry and eight hours; sign in again to resume observation.

Cloud sandbox provisioning, artifact/context transfer and scheduled cloud workflows
require separate ownership contracts. Existing local sandbox and policy behavior still
applies to every task. Source and acceptance commands are in
[cloud development](../develop/cloud-control-plane.md).

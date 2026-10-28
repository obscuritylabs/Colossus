---
title: Connect runtimes
description: Enroll Desktop or CLI runtimes, enable web shells, and share local conversations with Cloud.
audience: operator
type: how-to
icon: lucide/plug
---

# Connect runtimes

Connect Desktop or a configured CLI workspace to a Cloud project. The runtime
opens an outbound connection, so its machine needs no inbound public port. It keeps
its own tools, credentials, policy, and sandbox.

Select the project in Cloud, open **Fleet**, and create an invitation with the role
ceiling you intend to grant. Invitations expire after ten minutes and can be used once.

## Enroll Desktop

1. Select a Managed Local workspace and open **Control Plane** settings.
2. Enter the enrollment URL and paste the invitation.
3. Review the native confirmation of the Cloud origin and execution/approval access.
4. Check that the workspace appears online in Fleet.

Desktop keeps credentials in native custody. An external daemon target uses its
separately enrolled CLI connector. To remove Cloud authority, choose **Revoke
enrollment** in Desktop or **Revoke agent** in Fleet. Forgetting local data alone
does not revoke the enrollment.

## Enroll the CLI

From a configured workspace, run the enrollment URL shown in Fleet:

```sh
colossus cloud enroll https://colossus.example.com/api/enroll
```

Review the displayed Cloud origin, workspace, configuration, and local application
grant, then paste the invitation at the hidden prompt. The command prepares its own
protected connection references and dedicated local credential; you do not write
connection JSON or copy certificate fingerprints. The invitation never enters argv.
Piped input is also read from stdin.

The command starts a local worker and its outbound connector in the foreground.
Keep it running while using the runtime from Cloud. **Ctrl-C** checkpoints and stops
the worker it owns. To reconnect later, select the same workspace, configuration,
Colossus home, and enrollment name:

```sh
colossus cloud run
```

Use the ordinary CLI `--workspace` and `--config` options when selecting a runtime
explicitly. For several connections, choose a name at enrollment and reuse it:

```sh
colossus --workspace /work/project --config /secure/runtime.yaml cloud enroll \
  https://colossus.example.com/api/enroll --name engineering
colossus --workspace /work/project --config /secure/runtime.yaml cloud run \
  --name engineering
```

The saved name binds the workspace, configuration, local instance, and grant. A
changed or missing binding fails closed. Restore its matching private state, or
choose a new name for a deliberately different connection. Retrying an interrupted
enrollment uses the saved invitation and CSR rather than asking for another token.

By default, the dedicated application can execute, read, and control runs using the
configured `primary` role and its existing allowed tools, and answer user prompts.
Local policy and sandbox restrictions still apply. Policy approval responses and
human shells require separate explicit enrollment choices: `--allow-approvals` and
`--enable-human-terminals`. An existing name cannot silently gain either permission.

In another terminal, inspect or disconnect the connection:

```sh
colossus cloud status --name engineering
colossus cloud disconnect --name engineering
```

Disconnect leaves accepted work with the foreground worker. Use **Ctrl-C** in its
owning terminal when you want that worker to stop. Use `revoke` to remove Cloud
authority before `forget` or uninstalling; forgetting local enrollment alone does not
revoke the remote node. If revocation fails, retain the record so you can retry.
Certificates renew automatically; `cloud renew` also requests renewal.

### Trust a private enrollment endpoint

Publicly trusted HTTPS needs no extra option. For a local or private endpoint,
obtain its public trust-anchor certificate from the deployment operator and supply
it explicitly:

```sh
colossus cloud enroll https://colossus.localhost:9443/api/enroll \
  --enrollment-ca-file /secure/colossus-browser.pem
```

The file contains one public X.509 certificate, at most 16 KiB. A self-signed server
certificate can be the explicit anchor. TLS still verifies the hostname and
certificate validity; this option does not change OS trust or system DNS.

The native CLI routes `localhost` and names ending in `.localhost` directly to
loopback for enrollment, connection, renewal, and revocation, following
[RFC 6761](https://datatracker.ietf.org/doc/html/rfc6761#section-6.3). It bypasses
proxies for those names and literal loopback addresses while keeping the original
hostname for TLS verification. Other hostnames still use normal resolution and
proxy settings. Browser certificate trust remains a separate setup step.

Repeat the same trust option when retrying an interrupted enrollment. Its retained
exchange rejects a different origin, runtime, or trust anchor. Later `cloud run` and
certificate renewal use the separately enrolled connector CA and mutual TLS.

### Connect an independently managed worker

Use `--local-config` when you already manage a separate public worker and its
application grant. This advanced path leaves the worker's lifecycle with its
existing supervisor. The `control-plane` spelling remains available as an alias
for `cloud`; the standalone `colossus-connector` uses this external-worker path.

Use an existing runtime configuration and an owner-private public API directory.
The worker must be stopped while enrolling the local application.

### Grant the connector local access

Choose scopes, tools, and roles within the runtime's existing ceilings. This example
grants one connector application access to the primary role and two tools:

```sh
colossus --workspace /work/project --config /secure/runtime.yaml worker \
  --public-api-dir /secure/runtime-api \
  --enroll-application app:colossus-cloud-cli \
  --scope runs:execute --scope runs:read --scope runs:control \
  --scope prompts:respond --scope approvals:respond \
  --role primary --tool user.ask --tool filesystem.read \
  --credential-keyring-service dev.colossus.cloud \
  --credential-keyring-account engineering
```

Restart the worker using the same configuration and public API directory:

```sh
colossus --workspace /work/project --config /secure/runtime.yaml worker \
  --public-api-dir /secure/runtime-api
```

### Save the connector's local identities

Create an owner-private `/secure/connector.json` with the exact public identities
returned by enrollment. The bearer remains in the native credential store:

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

### Enroll in Cloud and start the connector

Run enrollment and paste the Cloud invitation at its stdin prompt. Then start the
connector with the same local configuration and enrollment name:

```sh
colossus control-plane enroll --local-config /secure/connector.json \
  --enrollment-url https://colossus.example.com/api/enroll --name engineering
colossus control-plane run --local-config /secure/connector.json --name engineering
```

Check that the workspace is online in Fleet. In another terminal, inspect the saved
enrollment with `colossus control-plane status --name engineering`.

Use `disconnect` to stop its connection while leaving the daemon and accepted work
alive. Use `revoke` to remove Cloud authority before `forget` or uninstalling. Retain
the enrollment if revocation fails so you can retry it. Certificates renew
automatically; `control-plane renew` also requests renewal. The standalone
`colossus-connector` accepts the same commands without the `control-plane` prefix.

## Enable a web shell

Remote human shells require an explicitly enabled CLI runtime. Desktop's local
Terminal setting does not enable remote access.

Before enabling shells, complete the Control Plane rollout on every replica. Older
replicas cannot parse the Terminal operator role or provide the relay. Linux workers
also need kernel pidfd support and readable `/proc` for ordinary job cleanup.

For a managed CLI runtime, select `--enable-human-terminals` on its first
`cloud enroll` command. This explicitly enables the owned worker and adds the local
connector's terminal scope. The human still needs the project's **Terminal operator**
role. A saved connection without shell authority requires a new enrollment name to
add it.

For an independently managed worker, enable all three access controls:

1. During offline application enrollment, add `--scope terminals:control` to the
   connector application's local grant.
2. Start the public worker with `--enable-human-terminals`:

   ```sh
   colossus --workspace /work/project --config /secure/runtime.yaml worker \
     --public-api-dir /secure/runtime-api --enable-human-terminals
   ```

3. Give the human account the project's **Terminal operator** role. Operator,
   Approver, Project administrator, and global administrator access do not include
   Terminal permission.

Once all three controls permit access, an online workspace advertises **Open Shell**.
Follow the [web shell workflow](work.md#open-a-shell-on-a-connected-host) to test it.

The shell runs as the worker's OS account in the selected workspace, with that
account's filesystem and network authority. Commands run outside agent approvals
and do not become model context. The browser cannot choose another executable,
starting directory, environment, or application credential.

**Detach** preserves the process for up to two minutes without an active reader;
reopening reattaches to it. **Close Shell** ends the process. Each terminal has an
eight-hour maximum lifetime. A worker permits eight sessions, with at most four per
human/application. Browser, connector, or Control Plane reconnection can retain the
shell while the worker survives. A worker or machine restart ends it; Colossus never
reruns its commands.

Output history is bounded. A fresh pane replays retained output and resize events.
If earlier history was evicted, it reports a gap. Unconfirmed input is never
automatically resent after disconnect.

## Run unattended

Supervise the managed `colossus cloud run` command under the selected OS account:
systemd on Linux, launchd on macOS, or your Windows service supervisor/Task Scheduler.
Keep the same workspace, configuration, private Colossus home, and enrollment name.
The command stays in the foreground and owns its worker; it does not install a
background service.

For the advanced external-worker path, supervise the worker and connector separately.
Preserve their private state directories and exact enrollment selector. Give
concurrent CLI daemon/connector pairs separate `COLOSSUS_HOME` directories; one native
vault has an exclusive process lease. Desktop manages its shared vault.

If the native credential store will not be unlocked, inject a random 32-byte hex
wrapping key through a protected process environment. Managed enrollment uses that
selector for both its local application credential and connector vault:

```sh
colossus cloud enroll https://colossus.example.com/api/enroll \
  --name engineering --vault-key-variable COLOSSUS_HEADLESS_KEY
colossus cloud run --name engineering --vault-key-variable COLOSSUS_HEADLESS_KEY
```

Retain the same wrapping key for every lifecycle command. A missing or changed key
fails closed; it never regenerates established credentials.

For an external worker, set
`--public-api-vault-key-variable COLOSSUS_HEADLESS_KEY` during worker enrollment and
hosting. Add this selector to the protected connector configuration:

```json
{
  "headless_authority": {
    "directory": "/secure/runtime-api/headless-credentials",
    "key_variable": "COLOSSUS_HEADLESS_KEY"
  }
}
```

Pass `--vault-key-variable COLOSSUS_HEADLESS_KEY` on connector enrollment and
lifecycle commands. A missing or changed wrapping key fails closed.

Use a supervised CLI daemon for persistent execution. Desktop's embedded connection
and Managed Local runtime stop when the app exits.

## Share local conversations

Local history stays private until its source application grants sharing. In Desktop's
**Control Plane** settings, choose **Share Desktop history for viewing** or
**Share history and allow continuation**, then save and confirm. The choice includes
existing and future sessions for that source application/workspace.

For a CLI application, use the source application's protected configuration:

```sh
colossus control-plane share-workspace --local-config /secure/source-application.json \
  --recipient-application-id app:colossus-cloud-cli --allow-continuation
```

Omit `--allow-continuation` for viewing only; use `--disable` to stop future sharing.
The source needs Read and Control, plus Execute for continuation. Imported source
runs stay read-only: sharing does not transfer approval or cancellation authority.
Disabling sharing does not erase history already synchronized to Cloud.

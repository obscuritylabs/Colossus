---
title: Enable hosted chat
description: Add shared browser execution with protected keys, approved connections, and an independent checkpoint authority.
audience: operator
type: how-to
icon: lucide/messages-square
---

# Enable hosted chat

Hosted chat gives each account a personal runtime in each project. API replicas
share SQL state and route work to one fenced execution owner. Browser affinity and
a shared filesystem are unnecessary.

Use [external PostgreSQL with verified TLS](configuration.md#use-an-external-database).
Hosted runtime journals do not accept the bundled database's plaintext exception;
the hosted values example disables bundled PostgreSQL.

## Prepare the external authority and keys

Deploy `colossus-chat-anchor` independently using the release-matched Control Plane
image. Give it a separate database/role, backup administration, storage-encryption
key, and trusted HTTPS ingress. Keep its storage key and database credential away
from the API/executor pods.

Its configuration references secrets through environment variables:

```json
{
  "bind": "0.0.0.0:8080",
  "trustedTlsIngress": true,
  "database": {
    "connectionVariable": "COLOSSUS_ANCHOR_DATABASE",
    "schema": "colossus_chat_anchors",
    "tls": { "kind": "webpki_roots" },
    "maxConnections": 4,
    "connectionTimeoutMs": 5000,
    "statementTimeoutMs": 5000
  },
  "bearerVariable": "COLOSSUS_ANCHOR_TOKEN",
  "storageKeyVariable": "COLOSSUS_ANCHOR_STORAGE_KEY"
}
```

`trustedTlsIngress` asserts that the non-loopback HTTP listener sits behind your
trusted TLS ingress. Restrict access to authorized executors. Use `custom_ca` TLS
when the authority's database uses private trust roots.

Provision independent random 32-byte encryption, signing, and wrapping masters
through your secret manager. Encode each as 64 hexadecimal characters without a
trailing newline; use the same format for the separate anchor storage key.
Every API/executor replica needs the same retained keys and anchor bearer token.
Create the executor Secret from protected files:

```sh
kubectl -n colossus create secret generic colossus-chat-state \
  --from-file=journal-key=/secure/cloud/chat/journal-key \
  --from-file=signing-key=/secure/cloud/chat/signing-key \
  --from-file=wrapping-key=/secure/cloud/chat/wrapping-key \
  --from-file=provider-key=/secure/cloud/chat/provider-key \
  --from-file=anchor-token=/secure/cloud/chat/anchor-token
```

The example shares a provider credential explicitly. Set `chat.providerKey` empty
and omit that Secret entry when connections use personal credentials only.

## Review the project configuration

Copy the chart's `examples/hosted-chat.yaml` to `hosted-chat-values.yaml`.
Replace its project ID, model, provider/MCP endpoints, and OAuth client settings.
Remove the sample MCP entry until its server and public S256 PKCE client are ready.
The MCP callback is `https://BROWSER_HOST/auth/mcp/callback`.

Hosted connections default to **Allow all configured connections**. Project
administrators can restrict provider, MCP, and Search origins through
**Project Admin → Network access**; see [project connection policy](../administration.md#control-project-network-access).
Global definitions can be published independently of any project's policy. The host
admits the configured, permitted connection origins when composing each runtime;
this does not grant arbitrary networking tools or filesystem/process authority.
Review any separate operator-configured `sandbox.networkDestinations` needed by
other allowed effects and the OAuth metadata/token servers used by the deployment.

Set `chat.existingSecret` and `chat.anchor.endpoint` to your external resources.
For a private anchor CA, set `chat.anchor.existingCaSecret`. The chart enables only
shared SQL execution with HTTPS anchors, never the single-host development mode.

## Enable and verify

```sh
helm upgrade --install colossus-control-plane \
  oci://ghcr.io/obscuritylabs/colossus-control-plane-chart \
  --version X.Y.Z --namespace colossus \
  -f deployment-values.yaml -f hosted-chat-values.yaml
kubectl -n colossus rollout status deployment/colossus-control-plane
```

Use two ordinary project accounts with different MCP identities. Start a chat,
answer an approval, reconnect through another replica, and continue the history.
Neither account should see the other's conversations, files, tokens, or usage.
Restart the execution owner and verify recovery; uncertain effects must not replay.
Controller readiness checks the database, so it does not replace these checks.

The example allows four resident runtimes per pod and a 4 GiB process/cache budget.
Measure before raising limits. Idle execution retires after 300 seconds by default;
active work and pending interactions retain ownership. Autoscaling down can interrupt
execution, so review disruption and scaling policies for your workload.

Back up shared SQL state, matching keys, and the independent anchor authority
separately. Restore must satisfy retained anchors. Existing single-host data is
not imported automatically; retain its volume and keys until a supervised migration.
See [backup and recovery](operations.md) and the
[developer guide](../../develop/cloud-control-plane.md#develop-hosted-personal-chat)
for source-only setup and account import.

---
title: Host the A2A application edge
description: Configure an authenticated text-only A2A listener over the shared Colossus application SDK.
audience: developer
type: how-to
---

# Host the A2A application edge

`colossus-a2a-server` is a separate application that connects to an enrolled local
daemon through `colossus-sdk`. Execution remains in that daemon's authorized runtime.
The listener stores no independent task or inbox database. The service boundaries and
delivery contract are defined in [ADR 0007](adr/0007-agent-communication.md).

The initial listener supports A2A wire version `1.0`, pinned to specification tag
`v1.0.1`, using JSON-RPC over HTTPS and server-sent events. It accepts text inputs and
releases text results. This source-built application currently requires Unix file
protection; the Windows configuration and private-key custody adapter is not implemented.

## Enroll an independent application for each peer

Use [trusted application enrollment](application-sdk.md#connection-and-enrollment)
to provision each peer's independent application credential into the platform
keyring. Keep the enrolled instance ID and leaf certificate pin as independent trust
anchors. Give each application only its intended role and tools, plus these scopes:

- `runs:execute` and `runs:read`;
- `agent_messages:send` and `agent_messages:read`;
- `runs:control` if the peer may cancel its tasks.

The incoming listener credential is a separate opaque bearer secret of at least
32 bytes. Configure only its lowercase SHA-256 digest. Distribute the secret through
your peer's credential custody channel. The listener never forwards that bearer to
the Colossus daemon. Each peer has a fixed operator-selected role and turn ceiling
between one and 100; incoming metadata cannot change those choices.

## Configure and launch the listener

Create an owner-private JSON file using absolute paths. The example values are
placeholders; `instance_id` must be the enrolled daemon's UUID and both SHA-256 values
must be the actual lowercase digests.

```json
{
  "bind": "127.0.0.1:8443",
  "public_url": "https://agents.example.com",
  "certificate": "/absolute/path/server-certificate.pem",
  "private_key": "/absolute/path/server-key.pem",
  "peers": [{
    "token_sha256": "<incoming-peer-token-sha256>",
    "daemon_descriptor": "/absolute/path/endpoint.json",
    "daemon_certificate": "/absolute/path/daemon-leaf.pem",
    "instance_id": "<independently-enrolled-instance-uuid>",
    "daemon_leaf_sha256": "<independently-enrolled-leaf-sha256>",
    "keyring_service": "colossus-a2a-example-peer",
    "keyring_account": "application-credential",
    "role": "primary",
    "max_turns": 8
  }]
}
```

The public URL must be an HTTPS origin. The configuration and private key must be
regular files owned by the current OS user, inaccessible to group and other users;
the listener rejects a final symlink. Its TLS listener accepts TLS 1.3. Keep the
certificate valid for the public hostname and route that origin to this listener.

From a source checkout:

```sh
cargo build --locked --package colossus-a2a-server
target/debug/colossus-a2a-server /absolute/path/listener.json
```

Startup verifies each authenticated daemon's message capabilities before accepting
requests. At most 32 peer profiles are configured. Concurrent requests and streams
are bounded to 32 globally and eight per peer, with bounded TLS handshakes and bodies.
Shutting down the listener releases its SDK connections; accepted daemon tasks retain
their durable execution lifecycle.

## Verify discovery and task submission

An authenticated `GET /.well-known/agent-card.json` returns the supported interface,
text modes, streaming capability, and bearer security requirement. Supply your bearer
through protected HTTP client configuration. Every RPC also requires the headers
`Content-Type: application/json` and `A2A-Version: 1.0`.

Send this JSON to `POST /`:

```json
{
  "jsonrpc": "2.0",
  "id": "request-1",
  "method": "SendMessage",
  "params": {
    "message": {
      "messageId": "peer-work-1",
      "role": "ROLE_USER",
      "parts": [{"text": "Review the repository's cancellation behavior."}]
    },
    "configuration": {"returnImmediately": true}
  }
}
```

The result contains a durable task ID and context ID. Repeating the same authenticated
peer's `messageId` and original request returns the same admission. Changed content
under that identity conflicts. A new message may address a live task by including
`taskId` and `contextId`; it enters that exact attempt's inbox and cannot increase its
turn ceiling or reopen a terminal task. A task identifier grants no authority.

`SendMessage` blocks until terminal work or local input is required unless
`returnImmediately` is true. `SendStreamingMessage` emits a task snapshot first,
followed by released artifact and status updates. A result artifact precedes its
terminal status. `GetTask`, `ListTasks`, `CancelTask`, and `SubscribeToTask` use the
same authenticated application ownership. List pagination binds its filters to a
frozen canonical journal head and orders by last lifecycle update; pages are capped
at 20. Final artifacts and recent input history are omitted from lists unless requested.

Use `GetTask` to reconcile a task if its listener connection ends. An uncertain
admission must be reconciled before another effectful submission; the listener never
automatically retries submission or cancellation. Read throttling receives bounded
backoff. Pending local approvals and prompts remain visible as input-required status
and require the existing authorized local response path.

## Supported profile

The listener rejects attachments, arbitrary data parts, remote content URLs, tenant
selection, protocol extensions, and push callbacks. It advertises only its supported
binding and does not expose private sessions, tool arguments, or reasoning.

Outgoing remote-agent calls remain an extension point behind
`colossus_policy::RemoteAgentClient`, the ordinary one-use effect permit and quarantined
result release. This feature does not register an outgoing URL-based tool or provide a
remote-agent adapter.

For local message operations, permissions and language SDK entry points, see
[Agent communication](application-sdk.md#agent-communication). For the rendered
inbox views, see [Inspect agent messages](../use/agent-communication.md).

# Colossus Python SDK

The Python SDK uses `grpc.aio` to connect to the authenticated loopback API.
Its organization-qualified distribution name avoids an unrelated project that owns the
normalized `colossus-sdk` name; the import namespace remains `colossus_sdk`.

```console
python -m pip install obscuritylabs-colossus-sdk==0.11.7
```

```python
from colossus_sdk import (
    ColossusClient,
    EndpointDescriptor,
    StaticBearerCredential,
)
from colossus.api.v1alpha1 import system_pb2

descriptor = EndpointDescriptor.from_json(descriptor_json)
credential = StaticBearerCredential(token_from_secure_enrollment)
client = await ColossusClient.connect(
    descriptor,
    leaf_certificate_pem,
    instance_id_from_trusted_enrollment,
    certificate_sha256_from_trusted_enrollment,
    system_pb2.DEPLOYMENT_MODE_SHARED_DAEMON,
    credential,
)

try:
    response = await client.agent_runs.get_run(run_id)
finally:
    await client.close()
```

The caller must provide the token directly. No helper reads credentials from an
environment variable, command-line argument, URL, or endpoint descriptor. The caller
must also provide an instance ID and certificate SHA-256 obtained independently during trusted
enrollment. Do not derive it from the mutable descriptor or certificate file; keep it
in signed application configuration or application-owned protected storage. The
channel verifies discovery and the leaf's explicit `BasicConstraints CA=false`
declaration against those anchors, disables
proxying and transparent retries, and applies authorization metadata only over TLS.
Before returning the client, `connect()` makes an authenticated
`SystemService.GetServerInfo` call and verifies the live instance ID, API package, and
expected deployment mode.

`decode_colossus_rpc_error(error)` exposes the bounded, typed Colossus detail from a
gRPC failure. It rejects malformed, oversized, duplicated, and non-canonical trailers
without resolving an `Any` type URL over the network. Its `retryable` field is
informational; an effect with an `unknown` outcome must be reconciled against durable
state before any caller-directed retry.

`client.agent_runs.watch_run()` resumes from the last exclusive cursor after an explicitly
retryable transport failure. On clean EOF it performs `GetRun` and succeeds only when
the run is terminal with `last_sequence` exactly equal to the watch cursor. It removes exact replays
and fails closed on sequence gaps. Only exact
`result`, `failure`, and `cancellation` updates stop the feed; a lifecycle `state`
notification does not. Effectful RPCs are never retried automatically.

Generate the `colossus.api.v1alpha1` modules with `../scripts/generate` before building
a wheel or source distribution. In a complete source checkout, the build backend
verifies both the schema/tool inputs against `../generated-inputs.sha256` and the
complete generated tree against `generated-output.sha256`. The source distribution
carries the generated tree, output manifest, and build support forward so its own wheel
build can repeat the output check without requiring repository-only schema inputs. The
SDK relies on `googleapis-common-protos` for the canonical `google.rpc.Status`; it never
packages a second `google.rpc` implementation.

### Agent communication

`client.agent_communication` uses the already authenticated SDK channel. Inspection
requires `agent_messages:read` and `runs:read`; sending requires
`agent_messages:send`. Enroll these scopes explicitly and check the runtime's optional
`agent_messages.read.v1` and `agent_messages.send.v1` capabilities.

```python
participants = await client.agent_communication.participants(root_run_id)
page = await client.agent_communication.messages(participants.participants[0].id)
for message in page.messages:
    print(message.id, message.receipt.state)
```

`send`, `get`, and `watch` validate bounded messages and receipts. The update feed has
an exclusive cursor independent of inbox pagination, and closing a watch releases
only its subscription. `submit_task_message` and `get_task` return validated curated
task snapshots; `list_tasks` returns a bounded generated page. Mutations are never
automatically retried. See [the application contract](../../docs/develop/application-sdk.md#agent-communication)
and [inbox receipt meanings](../../docs/use/agent-communication.md).

### Runtime policy metadata

`await client.agent_runs.get_runtime_policy_posture()` returns the immutable, typed
`RuntimePolicyPosture` dataclass. `decode_runtime_policy_posture` validates a released
response independently. Closed enums, safe model labels, and bounded tool/role lists
exclude private configuration. Configuration revisions use exact Python integers;
missing counters remain `None`. Unknown or duplicate fields, unsafe identifiers,
invalid UTF-8, oversized metadata, and unsupported schemas fail with a generic
`ValueError` without revealing the payload. The getter makes one authenticated read
and does not retry. This report describes native configuration and does not change
policy or independently attest its enforcement.

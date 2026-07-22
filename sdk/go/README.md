# Colossus Go SDK

The Go SDK establishes an authenticated TLS connection to a literal loopback Colossus
endpoint and provides durable run-watch cursor handling.

```go
descriptor, err := colossus.ParseEndpointDescriptor(descriptorJSON)
if err != nil {
    return err
}
credential, err := colossus.NewStaticBearerCredential(tokenFromSecureEnrollment)
if err != nil {
    return err
}
conn, err := colossus.Dial(
    ctx,
    descriptor,
    leafCertificatePEM,
    instanceIDFromTrustedEnrollment,
    certificateSHA256FromTrustedEnrollment,
    v1alpha1.DeploymentMode_DEPLOYMENT_MODE_SHARED_DAEMON,
    credential,
)
if err != nil {
    return err
}
defer conn.Close()

runs := v1alpha1.NewAgentRunServiceClient(conn)
```

Import generated messages from:

```text
github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1
```

Generated Go sources are checked in because Go modules are distributed directly from
version-control tags. Contributors regenerate them with `../scripts/generate` after a
contract change; the release gate is `../scripts/check-generated go`. That gate verifies
the committed `gen/` tree against `generated-output.sha256` before a tag is released.

`Dial` rejects plaintext and non-loopback endpoints and requires an instance ID and
certificate SHA-256 obtained independently during trusted enrollment. It verifies the mutable
discovery identity and the leaf's explicit `BasicConstraints CA=false` declaration
against those anchors before applying
a credential. Do not derive the expected pin from the descriptor or certificate file;
store it in signed application configuration or application-owned protected storage.
The client disables proxies, service-config retries, and generic gRPC retries and
applies a caller-supplied credential only as secure per-RPC metadata. The credential
cannot be supplied by a descriptor, URL, argv, or environment helper.
Before returning the connection, `Dial` makes an authenticated
`SystemService.GetServerInfo` call and verifies the live instance ID, API package, and
expected deployment mode.

`DecodeColossusRPCError` exposes the bounded, typed Colossus detail attached to a gRPC
failure. It rejects malformed, oversized, duplicated, and non-canonical details
without resolving an `Any` type URL. `Retryable` is informational; callers must
reconcile an effect with an `Unknown` outcome against durable state before replay.

`RunWatcher` accepts a small adapter around the generated `WatchRun` stream. It resumes
with the last exclusive cursor only after an explicitly retryable transport failure.
`RunWatchOptions.Reconcile` must perform `GetRun`; clean EOF succeeds only when it
reports a terminal run whose `last_sequence` exactly equals the cursor. The watcher
drops duplicate at-least-once deliveries and rejects
sequence gaps. For a generated run-update watcher, set `RunWatchOptions.IsTerminal` to
`colossus.IsTerminalRunUpdate[*v1alpha1.RunUpdate]`. Only exact `result`, `failure`,
and `cancellation` variants stop the feed; a lifecycle `state` notification does not.
No effectful RPC is automatically retried.

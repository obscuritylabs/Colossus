# Colossus language SDKs

The public SDKs are generated from the Protobuf contract in
`../api/colossus/api/v1alpha1`. They do not use the private worker protocol.

This directory contains:

- `typescript/` — Node.js/TypeScript package using native HTTP/2 gRPC.
- `python/` — asynchronous Python package using `grpc.aio`.
- `go/` — Go package using gRPC-Go.
- `buf.gen.yaml` — shared local code-generation template.
- `scripts/generate` — the only supported regeneration entry point.
- `scripts/install-codegen-tools` — installs exact generator versions locally.

The checked-in wrappers deliberately separate transport security from generated
messages. Protocol tests inject in-memory run streams; connector tests use an
ephemeral local TLS server and disposable credentials to exercise the complete pin,
authentication, and live-server-identity handshake without a running Colossus
installation.

## Security contract

An SDK connection has four inputs that must remain separate:

1. An owner-readable endpoint descriptor containing a literal loopback endpoint,
   API/instance identity, process metadata, and the SHA-256 pin of the public leaf
   certificate.
2. The public leaf certificate PEM, whose DER bytes must match the descriptor pin.
3. The expected instance identity and certificate SHA-256 provisioned independently
   during trusted enrollment.
4. A caller-supplied credential held only in memory.

Descriptors are not secret, but they must never contain a bearer token. SDKs reject
non-loopback endpoints, plaintext transports, malformed pins, nil instance UUIDs,
credentials containing whitespace or control characters, descriptor/expected-identity
mismatches, descriptor/expected-pin mismatches, and
certificate/expected-pin mismatches, missing Basic Constraints, and certificates that
do not explicitly declare `CA=false` before attaching a credential. Never derive the
expected pin from the descriptor or certificate file; keep it in signed application
configuration or application-owned protected storage. Credentials are redacted by
string and debug representations. They are never read from environment variables,
command-line arguments, URLs, or descriptor files.

A generic OS-keyring service/account pair is only a lookup namespace. Deployments that
must isolate credentials from hostile processes running as the same OS user need a
platform-specific application-bound credential provider and code identity. The built-in
generic provider does not claim same-UID process isolation.

The version-1 JSON descriptor has required `schema_version`, `api_version`,
`instance_id`, `endpoint`, `pid`, and `certificate_sha256` fields. Unknown fields are
rejected. `instance_id` is a canonical non-nil UUID, `api_version` is
`colossus.api.v1alpha1`, and `pid` is informational
process-discovery metadata—not an authorization primitive.

The generated API is `v1alpha1`. Applications should pin an SDK release. Each remote
connector makes authenticated `SystemService.GetServerInfo` its first read and refuses
to expose an effectful client until the actual instance ID, API package, and deployment
mode match trusted enrollment.

Each SDK exposes a bounded decoder for the canonical
`grpc-status-details-bin` trailer and `ColossusErrorDetail`. Decoders require one
exact type URL, cap the serialized status and detail sizes, bound user-facing text
and violations, and never resolve an `Any` URL or retry an RPC. A server-provided
`retryable` flag is advisory; an effect with an `unknown` outcome must be reconciled
against durable state before caller-directed replay.

## Regenerate

Install the pinned generators locally, then run:

```console
./sdk/scripts/install-codegen-tools
./sdk/scripts/generate
```

The generation path uses only local plugins: the Colossus API schema is never uploaded
to a remote code-generation service. Exact npm, Python, and Go generator versions are
declared in the package locks, requirements file, and installer. Generated trees are
replaced by Buf's `clean` mode; handwritten source lives outside those trees. Per-language
SHA-256 manifests bind every generated file and path to the freshly generated tree,
while `generated-inputs.sha256` binds generation to the schema, templates, exact tool
locks, and generation scripts. A release must run the language-specific checks after
regeneration. The `./sdk/scripts/check-generated` gate validates both layers before
packaging; the Python build backend independently enforces its output manifest.
Go bindings are committed because Go modules are fetched directly from version-control
tags; TypeScript and Python include their generated bindings in their registry
artifacts. Go deliberately uses gRPC-Go's canonical
`google.rpc.Status` dependency instead of generating a second copy whose global
Protobuf descriptor would conflict at process initialization.

`WatchRun.after_sequence` is an exclusive cursor. Every wrapper uses at-least-once
delivery, drops exact replays, rejects gaps, and reconnects the read-only watch stream
only after explicitly retryable transport failures. A clean server EOF is accepted
only after `GetRun` proves the same run is terminal and its `last_sequence` exactly
equals the cursor. Only the `result`, `failure`, and `cancellation` `RunUpdate` variants are
terminal; a lifecycle `state` update never ends the feed by inference. Effectful calls
are not retried automatically.

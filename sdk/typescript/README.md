# Colossus TypeScript SDK

This package connects Node.js applications to the authenticated loopback Colossus
gRPC API. It is not a browser SDK: a browser or WebView must call a trusted native
backend so bearer credentials never enter renderer memory.

Install the SDK version that matches the Colossus core release:

```console
npm install @obscuritylabs/colossus-sdk@0.10.3
```

```ts
import {
  StaticBearerCredential,
  createSecureGrpcClient,
  isTerminalRunUpdate,
  parseEndpointDescriptor,
  watchRun,
} from "@obscuritylabs/colossus-sdk";
import { AgentRunServiceClient } from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/agent_run";
import { DeploymentMode } from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/system";

const descriptor = parseEndpointDescriptor(descriptorJson);
const credential = new StaticBearerCredential(tokenObtainedFromSecureEnrollment);
const runs = await createSecureGrpcClient(
  AgentRunServiceClient,
  descriptor,
  leafCertificatePem,
  instanceIdFromTrustedEnrollment,
  certificateSha256FromTrustedEnrollment,
  DeploymentMode.DEPLOYMENT_MODE_SHARED_DAEMON,
  credential,
);
```

The token must be supplied directly by trusted application code. The SDK never reads
it from an environment variable, command-line argument, URL, or endpoint descriptor.
`createSecureGrpcClient` rejects plaintext and non-loopback endpoints and requires an
instance ID and certificate SHA-256 obtained independently during trusted enrollment.
It verifies the mutable discovery identity and the leaf's explicit
`BasicConstraints CA=false` declaration against those anchors
before a credential can be attached. Do not derive this argument from the endpoint
descriptor or certificate file. Store it in signed application configuration or
application-owned protected storage. The client also disables proxying and transparent
gRPC retries and attaches the credential as per-call metadata over TLS.
Before returning `runs`, the connector makes an authenticated
`SystemService.GetServerInfo` call and verifies the live instance ID, API package, and
expected deployment mode.

Use `decodeColossusRpcError(error)` to read the bounded, typed Colossus detail from a
gRPC failure. It rejects malformed, oversized, duplicated, or non-canonical status
trailers and never resolves an `Any` type URL over the network. A `retryable` result is
informational: never replay an effectful call whose outcome is `unknown` without first
reconciling its durable state.

Use `watchRun()` for a durable run feed. It reconnects only after an explicitly retryable
read transport failure, resumes with the last exclusive cursor, drops duplicate deliveries,
and rejects sequence gaps. On clean EOF it requires `RunWatchOptions.reconcile` to
perform `GetRun`; the returned run must be terminal with `last_sequence` exactly equal
to the watch cursor or the helper fails closed. Pass
`isTerminalRunUpdate` as its terminal predicate; only exact `result`, `failure`, and
`cancellation` variants stop the feed.

`npm pack` runs the generated-contract gate before compiling from a clean `dist/`
directory. The gate verifies both the schema/tool input digest and the exact generated
TypeScript tree recorded in `generated-output.sha256`, so a stale ignored binding tree
cannot be published accidentally.

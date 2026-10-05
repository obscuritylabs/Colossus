---
title: Develop and test the cloud control plane
description: Run the real local OIDC, browser, host and runtime connection stack and its acceptance checks.
audience: developer
type: how-to
---

# Develop and test the cloud control plane

The stack uses Rust for the host and connector, React/TypeScript with the shared Colossus
UI for the browser, existing redb/PostgreSQL journal adapters, and configurable OIDC.
`colossus-cloud` owns durable project/node/task operations. `colossus-cloud-protocol`
owns the closed versioned connection contract. `colossus-cloud-server` hosts HTTP/OIDC,
SSE and end-to-end TLS 1.3 mutual-TLS gRPC. `colossus-connector` uses only the public SDK
under a dedicated native application grant. Desktop embeds that connector; CLI exposes
it through `cloud`, and the standalone binary owns the same lifecycle commands.

## Run the local stack

Start the disposable identity provider and PostgreSQL fixtures:

```sh
docker compose -f deploy/cloud/local/compose.yml up -d
cargo run -p colossus-cloud-server --example init_local -- /private/tmp/colossus-cloud
cargo run -p colossus-cloud-server -- /private/tmp/colossus-cloud/cloud.json
```

On Linux, select an absolute owner-private path such as `/tmp/colossus-cloud`.
The example generates a private local CA and server key, plaintext development redb,
loopback HTTP on 8090 and mutual-TLS gRPC on 8443. It refuses to overwrite an existing
configuration. Development HTTP/plaintext allowances require explicit loopback-only
configuration and are rejected for production endpoints.

In another terminal:

```sh
npm ci --prefix apps/web --ignore-scripts --legacy-peer-deps --install-links
npm run dev --prefix apps/web
```

Open `http://127.0.0.1:5180` and sign in as `alice` with `local-only-password`. The fixture
realm/client and explicit subject membership match the generated local configuration.
Those checked-in credentials exist only for this disposable loopback fixture.

Generate an isolated credential-free Echo runtime:

```sh
cargo run -p colossus-cloud-server --example init_runtime -- /private/tmp/colossus-runtime
cargo build --locked -p colossus-cli -p colossus-connector --bins
```

Follow [runtime enrollment](../admin/cloud-control-plane.md#enroll-a-cli-daemon) with
the generated runtime config and API directory. Use `--tool echo` for the Echo fixture.
Cloud enrollment to the loopback host additionally requires `--allow-loopback-http`.
Start the worker and connector in separate terminals. The browser should show the
runtime ready, then show a submitted Echo task complete with its released output.

For Desktop, prepare current verified binaries before launching the native app:

```sh
cargo xtask desktop prepare --profile debug
npm run tauri --prefix apps/desktop -- dev -- --locked
```

Select an isolated Managed Local workspace and use its **Cloud** settings. Native
confirmation, keyboard/focus behavior, and actual platform credential storage require
an unlocked graphical desktop. Renderer fixtures do not establish those properties.

Build the combined Kubernetes image from the repository root:

```sh
docker build -f deploy/cloud/Dockerfile -t colossus-cloud:local .
kubectl kustomize deploy/cloud/kubernetes
```

The default image uses the release profile. `--build-arg PROFILE=dev` builds a stripped
debug image for local Linux acceptance. Builder/runtime images and local fixtures are
digest pinned. Production deployment and Secret configuration belong to the
[operator guide](../admin/cloud-control-plane.md).

Build and validate Linux Desktop in its disposable native builder:

```sh
docker build -f deploy/cloud/linux-desktop.Dockerfile --target artifacts \
  --output type=local,dest=/tmp/colossus-linux-desktop .
```

The host architecture selects x86-64 or ARM64. This exports an unsigned Developer
Preview Debian package and its checksum. The builder verifies the native masked
credential dialog under an isolated Xvfb display, including validation, clipboard
preservation and cancellation. The package seals its sidecar/CLI/search digests into
the main ELF executable. Linux packaging also runs in the full pre-merge acceptance
gate. A native Linux checkout with the builder's listed dependencies can use
`COLOSSUS_DESKTOP_RELEASE_CHANNEL=developer_preview COLOSSUS_DESKTOP_TEAM_ID=UNSIGNED
sh scripts/package-desktop-linux` after installing Desktop npm dependencies.

## Acceptance and completion checks

```sh
cargo test --locked -p colossus-cloud -p colossus-cloud-protocol
cargo test --locked -p colossus-cloud-server --lib
cargo test --locked -p colossus-credentials --test headless
cargo test --locked -p colossus-sidecar --test native_lifecycle -- --ignored
cargo xtask check web
cargo xtask check rust
cargo xtask check desktop
cargo xtask check docs
cargo xtask pr --base origin/main
```

The server acceptance owns a signed local OIDC issuer, real browser HTTP/SSE, encrypted
native enrollment records, the production mutual-TLS transport and connector, and a
real runtime using a deterministic credential-free model endpoint. It verifies prompt
responses, policy-bound file-write approval, cancellation, certificate rotation, durable
host restart, and revocation. No external model key or provider service is required.
The native sidecar acceptance requires a usable OS credential store and loopback sockets;
it proves independently granted cloud and Desktop callers cannot read each other's runs.

The ignored `managed_sidecar_cloud_run_isolated_and_application_exit_disconnects`
server acceptance uses `COLOSSUS_ACCEPTANCE_SIDECAR` to select a prepared native sidecar.
It signs in through OIDC, enrolls its independently bootstrapped cloud grant, submits
a browser HTTP task through the mutual-TLS connection, receives released native output,
and verifies application exit disconnects cloud readiness without changing the durable
run identity. The Linux package builder and native macOS/Windows pre-merge jobs run
this check with the platform credential store.

The operator-owned PostgreSQL variant runs the same transport/runtime/restart flow
against the local fixture. Inject `COLOSSUS_CLOUD_TEST_DATABASE` with its connection
URL and fresh independent 32-byte hex values for `COLOSSUS_CLOUD_TEST_JOURNAL_KEY` and
`COLOSSUS_CLOUD_TEST_SIGNING_KEY`, then run:

```sh
cargo test --locked -p colossus-cloud-server --lib postgres_oidc_to_runtime_and_recover -- --ignored
```

Each invocation owns a uniquely named `cloud_e2e_` schema in the disposable fixture
database. Never point this acceptance command at an operator's production database.

Repository tests additionally cover exact idempotent receipts, project isolation,
contiguous output and changed-duplicate rejection, role ceilings, admission bounds,
certificate renewal ACK reconciliation, OIDC signature/issuer/audience/nonce/expiry
validation and CSRF. The headless acceptance runs isolated processes to prove sealed
authority survives restart and fails closed when its wrapping key changes.

Use the actual web app for desktop/mobile, light/dark, keyboard/dialog and browser
console acceptance. Test native enrollment confirmation separately on the supported
platform. The Desktop gate includes the cloud web gate, and changes under `apps/web`
select that gate through the existing CI classification contract.

Keep local invitations, private keys, screenshots, logs and task plans in ignored
development output or owner-private temporary directories. Remove disposable fixtures
with `docker compose -f deploy/cloud/local/compose.yml down` after acceptance.

# Private HTTPS broker development component

This separate Rust workspace implements the next HTTPS request-origin boundary for
the [owned Chromium architecture](../../../docs/develop/adr/0008-owned-chromium-browser.md).
It is an OFF-default, Debug-only library with an explicitly sealed macOS development
child entry. Its optional native identity feature locally links the isolated
macOS owner; the ordinary library and child control protocol have no
certificate-store operations. It has no installer, CEF linkage or production
discovery path. A successful test does not establish separate-process admission,
private-key custody, complete macOS containment, or browser availability.

## Current interface and behavior

Trusted native composition supplies `Bootstrap`: one already-authorized canonical
HTTPS origin, at most eight exact pinned socket destinations with that origin's
port, reviewed public CA DER, and a fresh nonzero 256-bit enrollment generation.
This is an internal composition input, not a deserializable IPC request. DNS is
never resolved by this component. Literal-IP destinations must equal the admitted
IP; `localhost` destinations must be loopback. Importing a root does not admit an
additional origin or destination.

`BrokerLease::start` generates a fresh random proxy credential and a fresh
downstream CA/leaf certificate for only the admitted origin. The signing keys stay
inside this component; the CA signing key is dropped after issuing that leaf.
The lease publishes its private-native proxy endpoint and, only for its exact
active generation, the proxy credential, **public** downstream CA/leaf DER and
the SHA-256 of that exact leaf. The pin supplements normal CA and hostname
validation. It does
not publish upstream roots or accept a caller-supplied downstream key. No raw
signing IPC, client-key or key-export API exists. Local identity composition is
described below; it adds no identity operation to the control protocol.

An authenticated CONNECT must name the exact canonical host and port, with a
matching Host header. Downstream TLS checks SNI against the same origin, allowing
no SNI for literal IPs. Without a client identity, both TLS legs use rustls safe
defaults for TLS 1.2 and 1.3,
advertise only HTTP/1.1, and disable session resumption. Upstream TLS verifies
the admitted hostname, chain, signatures and validity against only the explicitly
reviewed roots. There is no dangerous verifier, opaque CONNECT relay or plaintext
fallback.

The broker admits a decrypted request only after its origin-form target and Host
agree with the enrolled origin. This check precedes every upstream socket. It
forwards one bounded HTTP/1.1 exchange per connection, replaces Connection with
close, and removes proxy authorization from both legs. Duplicate authority or
framing headers, CL/TE ambiguity, chunked uploads, upgrades, trailers, connection
header extensions, absolute-form targets and captured pipelining fail closed.
Later client bytes are never read or forwarded. HTTP/2, origin coalescing and
WebSocket byte tunnels have no supported path.

Responses support Content-Length, bodyless HEAD/204/304 and strict chunked framing.
Chunk sizes, CRLF boundaries, termination and the entire wire-byte budget are
checked before any response is released to the downstream TLS socket. Chunk
extensions, trailers, informational/upgrade responses, extra captured frames and
ambiguous or unframed bodies fail closed. Response buffering uses at most the
configured exchange budget per connection; request bodies are additionally
limited to 1 MiB. Headers are limited to 16 KiB and 100 fields. HTTP header/body
buffers and native proxy credentials are zeroized when dropped; diagnostics are
categorical and contain no HTTP or certificate bytes.

The native owner supplies immutable limits: at most 256 tracked connections,
24-hour absolute lifetime, one whole-exchange deadline and a 64 KiB to 64 MiB
per-direction byte ceiling. Authentication, slow TLS and partial bodies consume
the same connection bound. `revoke` stops admission, cancels every tracked socket,
and awaits drainage before returning success. Dropping a waiting future retains
the join handle for a retry. Drop schedules cancellation but cannot acknowledge
native process-tree or filesystem cleanup. An unknown task outcome remains an
owned obligation.

`start_on_loopbacks` requires the native supervisor's already-sealed nonzero
port and binds only `127.0.0.1` and `::1` at that exact port, with IPv6-only enabled
on the IPv6 socket. A collision on either address closes the partial allocation
and fails; there is no alternate-port or wildcard fallback. Both listeners share
one connection ceiling and drainage owner. The original `start` method remains
an isolated library test/development helper with an ephemeral IPv4 listener.

## Private control interface

`BrokerClient::from_admitted_stream` and `serve_control` adopt an already-owned
connected Unix stream, an independent zeroizing 32-byte authentication key and
one nonzero generation. **These APIs do not authenticate the kernel peer or its
signed package.** Native entry must independently admit that peer before it
delivers the key or invokes either API. A PID, a peer-supplied expected code hash,
possession of bootstrap bytes, or a private directory is insufficient. There is
no accepted installed entry point. The explicit development executable described
below adds fixed-channel parent admission before this control layer.

The closed binary protocol admits one immutable `BootstrapEnvelope`: exact HTTPS
origin, at most eight pinned destinations, at most eight reviewed public roots
with a total 1 MiB ceiling, immutable limits and the exact dual-loopback port.
IPv6 pins with nonzero scope or flow metadata are rejected; their identity is
never silently changed by encoding.
There are no file, descriptor, environment, executable, audit-port, identity or
signer nominations. Subsequent commands are only Status, Revoke and Shutdown.
Ready returns the private proxy credential and public downstream CA/leaf/pin;
upstream roots and every signing key stay in the broker.

Every frame uses domain-separated HMAC-SHA-256 over its role, strictly increasing
sequence, exact generation, bounded length and complete payload. The maximum
payload is 1 MiB plus 8 KiB; invalid bounds fail before payload allocation.
Unknown operations, trailing fields, reflected roles, replay, wrong generation,
wrong key and altered payloads close the control path. Payload and credential
buffers are zeroized. Owner requests are serialized; cancelling any exchange
fences the client permanently, so a late response cannot acknowledge a new
request. Bootstrap and response writes have five-second deadlines; immutable
lease expiry cannot be renewed by Status traffic.

Revoke and Shutdown acknowledgement is sent only after both listeners and every
tracked exchange positively drain. EOF, malformed authentication, expiry and
output failure also attempt bounded drainage. The server returns categorical
`DrainEvidence`, distinguishing no lease allocation, positively closed tracked
sockets and an unknown outcome. This evidence does not establish broker exit or
helper cleanup. Native supervision must retain and reap the exact broker process;
an interrupted or cancelled owner cannot infer cleanup from a lost control stream.

## Sealed macOS development executable

The separate `macos-https-broker-entry-development` feature builds
`colossus-native-browser-tls-broker` only on macOS in Debug. Its build script
requires `COLOSSUS_TLS_BROKER_PARENT_CDHASH`, exactly 40 hexadecimal characters
representing a nonzero 20-byte hash from independently verified native parent
metadata. This is a **build-only** input embedded in the child image. Missing,
malformed and zero seals fail compilation; release and other-platform entry
builds are rejected. No runtime environment variable or file can replace it.

The real supervisor must use
`colossus_darwin_process::spawn_suspended_parent_channel`, retain the exact stopped
child immediately, verify its live code against independently trusted broker
metadata, and then resume it. The child accepts no command-line arguments. Before
creating its async runtime, it consumes the fixed inherited descriptor three once
through `take_connected_parent_channel`. The Darwin primitive verifies a connected
Unix stream, close-on-exec ownership, kernel socket-peer PID and UID, the actual
unprivileged direct parent, its exact live process incarnation and the compiled
code requirement. It reads no bootstrap bytes during admission.

Only after admission and another retained-peer check may the parent deliver the
fixed 64-byte private entry frame: an independent nonzero 32-byte key followed by
the nonzero 32-byte generation. Equal key/generation values are rejected. The child
reads it with a five-second deadline, zeroizes the frame, rechecks the retained
parent and begins the authenticated control protocol. It revalidates parent
incarnation/code every 250 milliseconds while serving. Parent change or loss
fences service and exits with unknown drainage; cancellation cannot issue a
positive socket or native shutdown acknowledgement. The exact process owner must
reap and reconcile it. No audit-session right is received or joined by this entry.

Standard output remains empty. Standard error emits at most one fixed categorical
line; credentials, certificates, keys, payloads and native error strings are never
printed by the entry. Exit zero means authenticated Shutdown acknowledgement was
sent after positive tracked-socket drainage. Status 10 rejects arguments, 11
rejects parent admission, 12 reports interrupted entry bootstrap, 13 rejects
entry authentication, 20 reports control EOF, 21 reports expiry, 22 reports a
rejected control message, 23 reports interrupted control and 24 preserves unknown
drainage. These statuses never prove descendant cleanup or installed acceptance.

For a reviewable development build, first build/sign the specific trusted parent
and independently verify its hash. Set `verified_parent_cdhash` from that reviewed
metadata, then retain a fresh private build directory:

```sh
task_tmp="$(mktemp -d /private/tmp/colossus-tls-broker-build.XXXXXX)"
broker_build="$task_tmp/target"
COLOSSUS_TLS_BROKER_PARENT_CDHASH="$verified_parent_cdhash" TMPDIR="$task_tmp" \
  cargo build --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --target-dir "$broker_build" --bin colossus-native-browser-tls-broker \
  --features macos-https-broker-entry-development
codesign --force --sign - --timestamp=none \
  --identifier com.obscuritylabs.colossus.tls-broker-development \
  "$broker_build/debug/colossus-native-browser-tls-broker"
codesign --verify --strict "$broker_build/debug/colossus-native-browser-tls-broker"
codesign -d --verbose=4 "$broker_build/debug/colossus-native-browser-tls-broker"
```

This command produces an ad hoc development image, not a publisher-approved
package. Stage it outside Core/browser filesystem grants, record its post-signing
hash in the native supervisor's independently verified immutable package metadata,
and revalidate exact live broker code before any key delivery. The parent seal
must not come from the unauthenticated stream; the parent's broker requirement
must not be self-reported by the broker. A signed package policy can bind those
two roles without circular compiler hashes. That policy, broker OS containment,
actual two-process admission/rejection, parent-crash reconciliation and installed
signing remain separate acceptance requirements. Running this child directly from
a shell is not a supported launcher or an admission test.

## Later native composition

The accepted platform supervisor must launch a distinct, authenticated broker
executable and retain its exact code/process identity and private filesystem
boundary. Its owner must deliver the already-authorized immutable envelope over
private native bootstrap and bind cancellation/cleanup to the same generation.
The inherited-stream entry must validate an independently sealed signed-package
peer policy, including kernel socket-peer identity, before reading its key or
envelope. This avoids circular Core/broker compiler hashes and any expected hash
provided by an unauthenticated peer. The native supervisor must preseal the exact
dual-loopback port and compose `start_on_loopbacks` with the fixed macOS main/helper
policy. The library and control tests alone do not prove this native admission.

The separately supervised CEF host receives the exact proxy endpoint, private
proxy credential and public downstream leaf through authenticated private
bootstrap. The guarded Mac development adapter admits only that exact leaf for
the enrolled HTTPS origin and bounded lifetime, and only when CEF reports the
sole certificate error as an unknown authority. Name, validity and other
certificate errors remain fatal. The broker independently performs normal
upstream CA and hostname verification. No default/search-list/personal Keychain,
OS trust or certificate-ignore flag is changed. Browser,
network service and every helper must inherit the proxy-only OS boundary; broker
origin checks cannot replace direct TCP/UDP/DNS/QUIC/WebRTC denial. Core's effect
permit, origin review, audit and result-release gateway remain the authority owner.

The optional `macos-native-identity-development` feature concretely links the
[owned native identity](../pki/mac/README.md) through a narrow safe Rust adapter.
Trusted in-process composition retains an actually prepared native owner through
startup errors and cleanup, borrows it into
`ClientIdentity::from_mac_native` and then `start_with_identity` or
`start_with_identity_on_loopbacks`. The immutable generation, exact canonical
origin and reviewed public leaf fingerprint must match. The separate broker
keeps the nonexportable key outside every Core/Root/browser filesystem grant.
Identity objects, native pointers, mutation callbacks and signing commands never
cross its control protocol.

After decrypted HTTP admission, the upstream resolver obtains one native
handshake token bounded by the exchange deadline and thirty seconds. Rustls
retains its normal server CA/hostname verification before the private-key
operation. The signer accepts only the fixed RFC8446 TLS1.3 client
CertificateVerify context and a SHA256 transcript, consumes the token on every
attempt including errors, and supports P256/SHA256 or RSA-PSS/SHA256. Identity
connections restrict both protocol and cipher negotiation to TLS1.3 with SHA256
transcripts; TLS1.2, SHA384 transcript suites and arbitrary messages fail closed.
Task drainage precedes explicit native cleanup. Unknown native cleanup remains
retryable and never produces a successful revocation result.
Native Security signing is synchronous: the deadline fences accepted output,
but Tokio timeout/abort cannot interrupt a stuck native operation. Independent
supervision must bound that broker process and preserve unknown physical state
after a native timeout or forced termination.

Native enrollment requires an independent creator-only mutation authority and
positive exact database/lock generation receipts. Native retirement requires
the same independent exclusion through exact removal and durable parent fsync;
mode bits, advisory locks and
caller assertions cannot implement that boundary. The production supervisor
implementation, actual item/key denial and real native mTLS/cleanup acceptance
remain absent. The feature is Debug-only and does not enable the installed entry,
CEF transport or production discovery.

The separate `macos-native-custody-development` feature constructs the concrete
[native mutation authority](../pki/mac/README.md) only after independently sealed
Core admission. Its public package trust is embedded at build time from
`COLOSSUS_TLS_BROKER_MAC_PACKAGE_TRUST`. It derives fixed own-package paths and
checks exact signed inventory, role-policy source bindings and reviewed native
file-denial evidence before opening the retained custody parent. A missing
signed five-role report denies admission before any Keychain allocation. No
identity, policy, raw descriptor, callback, signer or trust-key field is added to
the control protocol. Actual identity enrollment, item/key denial, native mTLS
and removal still require native acceptance; the ordinary four-case TLS entry
without this feature continues to require only its existing parent seal.

Actual Core/private-channel/CEF-public-CA, server-observed mTLS, foreign-process
key denial, broker/parent crash recovery, full-tree cleanup and signed installed
component acceptance remain necessary before availability changes. The ordinary
opaque `browser_egress` lease stays unchanged and does not gain request-origin
enforcement from this library.

## Focused verification

From the repository root, with the pinned Rust toolchain and permission for
ordinary disposable loopback sockets:

```sh
cargo fmt --manifest-path native/browser/tls-broker/Cargo.toml --check
cargo clippy --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --all-targets --features macos-https-broker-development -- -D warnings
cargo test --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --features macos-https-broker-development
cargo check --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --no-default-features
```

The library tests create memory-only private CAs, ordinary connected Unix stream pairs
and numeric IPv4/IPv6 loopback TLS
servers. They verify successful HTTP and fragmented 128 KiB chunked downloads,
normal TLS 1.2 compatibility, upstream CA/hostname/expiry failures,
CONNECT/SNI/decrypted-Host rejection before
upstream access, credential/generation fencing, malformed or ambiguous framing,
byte/deadline/connection limits, exact dual-loopback binding, exported leaf pin
matching an actual normally verified TLS handshake, authenticated control replay/
reflection/tamper rejection, cancellation fencing, owner EOF, expiry and
cancellation-safe drainage. Six client-identity tests additionally prove
server-observed mTLS via the local signer, exact origin rejection, hostname
failure before signing, TLS1.2 denial, fixed client transcript parsing, one-use
signing and retryable native-cleanup contracts with a memory-only test backend. They
create no OS-store items or native browser processes and call no Darwin audit
port, native UI or CEF APIs.

On macOS, compile and exercise the concrete native linkage without creating any
store or executing a native entry:

```sh
cargo clippy --locked --manifest-path native/browser/pki/mac/Cargo.toml \
  --all-targets -- -D warnings
cargo test --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --features macos-native-identity-development
```

Compile-only verification of the child target may use the conspicuous dummy seal
`0101010101010101010101010101010101010101`; it admits no real parent and must never
be represented as native process evidence. With that build-only seal, the entry's
pure authentication test adds one test:

```sh
COLOSSUS_TLS_BROKER_PARENT_CDHASH=0101010101010101010101010101010101010101 \
  cargo clippy --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --all-targets --features macos-https-broker-entry-development -- -D warnings
COLOSSUS_TLS_BROKER_PARENT_CDHASH=0101010101010101010101010101010101010101 \
  cargo test --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --all-targets --features macos-https-broker-entry-development
```

These commands execute ordinary protocol/TLS tests and a pure entry-frame test;
they do not execute the native child entry. Actual signed-child startup, wrong parent
hash rejection and crash testing belong to an explicitly supervised native
acceptance fixture, not these library/protocol commands.

## Signed two-process acceptance fixture

`scripts/run_native_acceptance.py` prepares a separate macOS Debug toy parent and
two broker images. Preparation builds and ad hoc signs the parent first, obtains
its actual code hash, then compiles the real broker against that independent
parent hash. The second broker intentionally requires a different parent hash.
Separate Cargo target directories prevent either compilation from replacing the
other image. The retained 0600 receipt records post-signing digests, code hashes,
source fingerprints and exact public operator commands. Neither authentication
keys nor proxy credentials are written to that receipt or any argument/environment.

With Rust 1.96 selected, prepare without launching a native fixture:

```sh
python3 -B native/browser/tls-broker/scripts/run_native_acceptance.py \
  --prepare-only --output-root /private/tmp/colossus-tls-native-evidence
```

The output root must be owned and 0700; the preparer creates a fresh retained
allocation within it. It signs only with the ad hoc `-` identity, creating no
Keychain or certificate-store items. After reviewing the printed `prepared.json`,
an explicitly authorized native test operator may run the recorded commands:

```sh
python3 -B native/browser/tls-broker/scripts/run_native_acceptance.py \
  --run --prepared /private/tmp/colossus-tls-native-evidence/tls-native-REPLACE/prepared.json
```

`--case positive`, `--case eof`, `--case wrong-parent` or `--case parent-crash`
selects one exact case. Cases refuse reuse of existing evidence. Preparation is
the default; it never invokes either native fixture image. Native runs have a
120-second outer deadline and retain every allocation/log, including failures.
The outer Python process owns only its direct fixture; a timeout cannot substitute
for the Rust fixture's broker/parent cleanup evidence.

The parent fixture accepts public operator launch metadata only. It retains each
stopped child before checking its exact live code against the prepared broker
hash, then resumes it and sends fresh independent private key/generation bytes.
Its one synthetic localhost TLS origin uses a memory-only private CA. Actual
HTTPS crosses the distinct broker process through IPv6 loopback, with normal
downstream CA/hostname verification and exact leaf-pin observation; the pinned
upstream observes the request without proxy credentials.

| Case | Required native observations |
| --- | --- |
| `positive` | Verified TLS/request/pin, authenticated Shutdown acknowledgement, both listeners and a held partial socket closed, exact direct broker reaped with status 0, empty stdout/stderr. |
| `eof` | Verified TLS/request/pin, owner control stream closed, both listeners and held socket closed, exact direct broker reaped with categorical status 20; no Shutdown acknowledgement. |
| `wrong-parent` | Live broker image matches its independent prepared hash, but its compiled parent requirement rejects the actual parent before Ready; status 11, fixed categorical stderr, no TLS availability, both ports denied. |
| `parent-crash` | Signed outer controller enrolls the live broker and registers an exact kernel exit observer before authorizing the admitted parent to call `process::exit(73)` with Rust owners still live. Controller reaps only that parent, observes the broker's exact process exit with status 20 or 24, and verifies held-socket closure and both endpoints denied. It never claims to reap the grandchild or receive a Shutdown acknowledgement. |

The crash-control channel has separate role/generation/sequence HMAC framing and
contains only closed Ready/Crash/Ack messages. The outer observer remains attached
to the original kernel process incarnation after reparenting or PID reuse. A
timeout/error may signal only the retained genuine identity, records forced or
unknown evidence and fails acceptance. No audit-session port is received, joined,
forged or probed. These tests establish only the explicitly observed broker and
socket facts; every receipt keeps `full_helper_cleanup_proven` and
`production_enabled` false.

The preparer's ten pure Python tests mock build/sign boundaries and reject changed
images, command tampering, nonprivate receipts, source mutation and misleading
cleanup/TLS results. The example's five pure tests also check HTTP field-name
casing without accepting an X-Host or duplicate authority, and preserve original
exchange failures alongside separate cleanup categories. A completed exact-child
reap is retained before independent output-channel drainage. These argument,
request-observation and controller-stream tests may
be run without invoking its native main:

```sh
python3 -B -m unittest discover -s native/browser/tls-broker/scripts -p 'test_*.py'
cargo clippy --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --example native-parent-acceptance --features macos-https-broker-native-acceptance \
  -- -D warnings
cargo test --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --example native-parent-acceptance --features macos-https-broker-native-acceptance
```

The fixture requires actual native execution to claim a pass. Its source, mocked
preparation and successful compilation alone do not establish parent admission,
crash cleanup, Core/CEF trust integration, private client-identity custody, broker
OS containment or signed installed acceptance.

On October 11, 2026, the signed ARM64 Debug fixture passed all four native cases
using the recorded commands above. The admitted parent code hash was
`eb24df4c654075b906049ec14372db870bbdd8ce`; the broker code hash was
`775b11bcd1992265c1c86b890cb146f0f9348eab`. Positive shutdown reaped the exact
broker with status 0 after its authenticated acknowledgement. Owner EOF reaped
it with status 20. The wrong-parent image exited with status 11 before TLS
availability. Parent crash reaped the sacrificial parent with status 73 and
observed the broker's exact kernel exit with status 20; it did not reap that
grandchild or receive a Shutdown acknowledgement. Both loopback endpoints and
the held partial socket closed, without forced termination. The real HTTPS cases
observed normal CA/hostname verification, exact leaf pin and the upstream request
without proxy credentials. Retained private result receipts and logs contain
these categorical facts; every receipt preserves the separate whole-helper and
production flags as false. This result does not establish Core/CEF trust,
client identities, broker OS containment or installed distribution acceptance.

`cargo xtask check rust` includes this independent workspace's ordinary library
feature tests, formatting, strict Clippy and disabled-default check. The dependency
gate also checks its separate lockfile. Native entry seals, the explicit fixture
feature, Python preparation checks and actual signed-child launches retain the
focused commands above; the ordinary completion gate cannot establish native
acceptance or production availability.

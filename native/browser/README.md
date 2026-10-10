# Native Chromium feasibility component

This component embeds Chromium through pinned CEF. Its native-only C ABI hosts
real browser objects, isolated request contexts, and native child HWND/NSView
surfaces, and executes DevTools methods on those objects without a debugging TCP
listener. It is a developer feasibility component; release modes remain false.

Connected Core/worker/SDK/Desktop control is implemented in source, with a separate
native credential and presentation relay. It remains unavailable in ordinary builds
without an accepted publisher package. Same-page agent-to-human takeover, protected
persistent-profile management and multi-origin human SSO are unfinished. macOS
separate-host containment and owned certificate custody are unsupported;
Windows native/broker acceptance is outstanding. The corrected OCI `sysinfo`
policy passed actual Core open/snapshot/run-finish, authenticated read-only
presentation/detach, five-host profile restart/reset and per-request PKI fixtures
with graceful physical cleanup and unchanged component checks. These results do
not enable production containment or shipping modes. See the
[current implementation and release boundary](../../docs/develop/adr/0008-owned-chromium-browser.md#current-enablement-and-remaining-work).
Successful macOS human preview evidence retains its preview scope.

The Linux fixture demonstrates no-display browser creation, navigation, DOM
inspection, native mouse events, a subsequent changed DOM observation, PNG
capture, negative target/URL/parameter controls, and shutdown. It uses windowless
Alloy rendering and the Ozone headless platform. The Chromium sandbox remains
enabled. The Chrome-style `--headless` approach did not complete browser creation
in the initial test and is not used here.

## Build and verify

Acquire the [pinned sources](component/README.md) first. From the repository root:

```sh
cef_source=$(python3 -B native/browser/scripts/component.py fetch --platform linux64)
cmake -S native/browser -B .local/cef-build -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCEF_ROOT="$cef_source"
cmake --build .local/cef-build --parallel 4
cmake --install .local/cef-build --prefix "$PWD/.local/cef-component"
python3 -B native/browser/scripts/component.py inventory \
  --root .local/cef-component --platform linux64 \
  --executable colossus-browser-probe
python3 -B native/browser/scripts/run_probe.py --component .local/cef-component
```

Run this on an unprivileged Linux native host that permits Chromium's namespace
and seccomp sandbox. The command sandbox in some managed development runners
blocks syscalls required by Chromium; use the authorized native acceptance lane,
without disabling Chromium's sandbox. The harness uses a fresh private temporary
profile, removes display variables, serves only the synthetic local fixture, and
reaps its process group. No shared developer profile or ambient browser is used.
When rebuilding an already inventoried stage, restage into a new directory before
creating its new inventory. Inventory never establishes publisher authenticity.

CMake requires the locked CEF version and refuses `USE_SANDBOX=OFF`. The staged
Linux component depends on glibc and the platform libraries used by CEF, including
NSS, ATK, X11 runtime libraries and audio libraries; it is separate from the musl
Colossus CLI executable. Its installed size is substantial: the first Linux
feasibility stage occupied approximately 1.5 GiB, before packaging measurements.

## Standalone authenticated Linux host acceptance

The separate `native/browser/driver` Rust workspace implements an actual CEF host
and the same private authenticated bridge as Core's browser adapter. Its ordinary
build fails explicitly as unavailable and does not download or link Chromium.
To build and stage the linked source component after the CMake build above:

```sh
export COLOSSUS_CEF_ROOT="$cef_source"
export COLOSSUS_CEF_NATIVE_LIB_DIR="$PWD/.local/cef-build"
export CARGO_TARGET_DIR="$PWD/.local/native-host-build-linux"
cargo build --locked --manifest-path native/browser/driver/Cargo.toml
cmake -S native/browser -B .local/cef-build \
  -DCEF_ROOT="$COLOSSUS_CEF_ROOT" \
  -DCOLOSSUS_BROWSER_HOST_EXECUTABLE="$CARGO_TARGET_DIR/debug/colossus-native-browser-host"
host_stage=$(mktemp -d "$PWD/.local/cef-host.XXXXXX")
cmake --install .local/cef-build --prefix "$host_stage"
python3 -B native/browser/scripts/component.py inventory \
  --root "$host_stage" --platform linux64 --executable colossus-native-browser-host
python3 -B native/browser/scripts/run_host_probe.py \
  --component "$host_stage" --host "$host_stage/colossus-native-browser-host"
```

The harness verifies all native startup phases before requesting authenticated
readiness and requires the host's entire process group to disappear after normal
shutdown. A development environment whose PID 1 does not reap adopted helpers
must run the same fixture under an installed process subreaper, for example
`tini -s -- python3 -B native/browser/scripts/run_host_probe.py ...`. Exited helper
zombies do not satisfy the cleanup check. Linux component verification also checks
the host's ELF dependency order: Chromium's `libcef.so` must precede `libc.so.6`
so its supported syscall interposers can resolve their next libc implementation.

The fixture creates three anonymous inherited Unix sockets: a bounded native-only
bootstrap carrying enrollment and authentication material, a typed data channel,
and an independent cancellation/close channel. The main thread owns CEF and pumps
its event loop while bounded queues connect the authenticated async endpoint.
Helpers dispatch through the supported CEF entry before inspecting parent-only
channels. Parent channels are close-on-exec. Credentials never enter process
arguments, logs, page host objects, or a debugging listener.
The native argument vector retains its string storage and a null sentinel after
the actual argument count. Dedicated Unix hosts set a private process file mask
before helpers or CEF threads start, including Chromium-created download files.

The supervisor may retain the bootstrap channel's read half for categorical
startup diagnostics. At most eight five-byte `CBH\x01` records report phases 1–7
or a failure's high bit and last phase. They contain no configuration or page
bytes and do not establish capability acceptance. An optional fourth private
channel carries authenticated bounded OSR frames and typed presentation input;
it uses a distinct derived key and never exposes raw DevTools authority. The
original three-channel headless entry remains supported.

Actual native acceptance checks semantic snapshots, native field input and clicks,
password-target rejection, single-option selection with observed DOM verification,
safe key presses with observed field changes, scrolling with a page event receipt,
bounded load and element waits, navigation history, reload, idle stop, and owned
tab allocation, selection, and closure. It also checks closed-tab denial, the fixed
proxy's authentication, exact-origin denial, and acknowledged native close plus
`CefShutdown`. References retain exact document and snapshot ownership;
navigation invalidates them both before the request and after main-frame commit,
including snapshots taken while the response was pending. All mutation clears
prior snapshot references. Select supports ordinary single
selection with at most 256 options and rejects multiple selection explicitly.

Add `--screenshot` to the inherited-channel host fixture to require an actual PNG
capture. It verifies the fixed viewport, complete SHA-256, PNG chunk checksums and
decoded pixels, ordered private chunks of at most 64 KiB, and one-shot transfer
retirement. Captures remain native custody until the runtime's post-effect output
release and artifact publication succeed; PNG bytes do not enter observations.

This remains developer acceptance: the synthetic enrollment exercises private
transport validation, and the result explicitly reports
`production_containment: false`. It does not change the component's false release
modes. Proxy configuration, native URL checks and the Chromium renderer sandbox do
not prove whole-host network confinement. Production Core/CLI composition requires
publisher-authenticated component evidence, a contained process supervisor, complete
egress acceptance, and cancellation that also revokes and drains the proxy lease.

## Publisher-bound offline Linux CLI package

Ordinary CLI and worker builds currently have no accepted browser distribution
and report browser automation unavailable. The source probes above do not enable
it. A publisher must first accept the exact installed Linux component, immutable
runtime image, complete process-tree and egress containment, typed actions, and
acknowledged cleanup. An administrator must then explicitly preload that accepted
image into the local Docker engine. Neither CLI startup nor the package builder
downloads Chromium, loads images, signs releases, or promotes acceptance flags.

The optional browser package uses the existing offline-bundle publisher trust in
[`release/bundle-publisher.json`](../../release/bundle-publisher.json). Its signed
`BundleManifest` is named `colossus-owned-browser-linux-release` and binds exactly
these three files, in order: `acceptance.json`, `browser-release.json`, and
`component/browser-component.json`. Each entry includes its exact size and SHA-256.
The publisher signature is verified both before compiler embedding and during
runtime discovery. The binary embeds those exact signed manifest bytes through
the build-only `COLOSSUS_CLI_BROWSER_MANIFEST` selector; runtime environment,
configuration, model input, and command arguments cannot select a browser package.

The managed sidecar uses the same strict verifier and can independently embed an
accepted manifest with the build-only `COLOSSUS_SIDECAR_BROWSER_MANIFEST` selector.
Both build seals reject a manifest when Cargo's target differs from
`x86_64-unknown-linux-gnu`. A publisher places the authenticated payload at the
sidecar executable's fixed adjacent `browser/` directory. The workspace-attested
sidecar discovers it before opening the worker runtime, and the worker retains
the native supervisor through setup, serving, errors and cancellation. Successful
exit requires acknowledged browser shutdown. There is no browser selector in
Desktop's inherited application configuration. This currently composes Headless
automation only; Desktop's private presentation attachment is connected in source.
Accepted Windows or macOS Embedded distribution and native platform execution
remain release requirements.

An accepted payload directory has this exact layout:

```text
accepted-payload/
  manifest.json
  acceptance.json
  browser-release.json
  runtime-image.tar
  component/
    browser-component.json
    colossus-native-browser-host
    ...every file and directory in the component inventory
```

`browser-release.json` contains `schema_version: 1`,
`purpose: "colossus-owned-browser-linux-release"`,
`target: "x86_64-unknown-linux-gnu"`, the immutable `image_id` (`sha256:` plus
64 lowercase hexadecimal digits), `image_archive_sha256`,
`component_manifest_sha256`, and `capabilities`. The accepted capabilities require
available Headless mode only, restrictive egress, an exact CEF engine version,
unique supported actions, and positive limits within the compiled `BrowserLimits`
ceiling. This initial ordinary CLI package declares `private_ca_trust: false` and
`client_identities: false`; the diagnostic native NSS workflow does not create a
CLI credential-enrollment interface.

`acceptance.json` binds the same schema version, target, component digest, image
ID, and image-archive digest. The publisher's independently reviewed receipt must
have `chromium_sandbox_verified`, `process_tree_containment_verified`,
`egress_denial_verified`, `typed_actions_verified`, `cleanup_verified`,
`installed_browser_package_verified`, and `production_acceptance` all true.
Missing, false, unsigned, or mismatched receipts are rejected. Editing a source
component's false modes or constructing a synthetic fixture does not satisfy
publisher acceptance. Component inventory modes must independently be Headless
true and Desktop false. Every component file is rehashed and its exact mode,
size, normal relative path, and exclusive regular-file ownership are checked;
links, extra unlisted files/directories, group/world-writable files, and oversized
payloads are rejected. The runtime-image archive is also rehashed.

After the publisher provides that accepted payload, build an offline package as
a non-root user with the repository's already provisioned Cargo dependencies:

```sh
python3 -B native/browser/scripts/stage_cli_browser_bundle.py \
  --payload /absolute/path/to/accepted-payload \
  --build-cli --destination "$PWD/.local/colossus-browser-package"
```

The destination must be new. `--build-cli` performs an explicit offline, locked
release build with the publisher manifest embedded and a separate build directory.
Alternatively supply `--cli /absolute/path/to/already-sealed/colossus`; the builder
requires that executable's embedded manifest to match the payload exactly. It
copies only authenticated bytes into a fresh private staging directory, repeats
verification on the copy, and publishes this layout by renaming the owned stage:

```text
colossus-browser-package/
  colossus
  OFFLINE-BROWSER.txt
  browser/                    # the exact accepted-payload layout above
```

The package's CLI signing and publication remain separate publisher actions.
Read-only verification commands do not open a runtime or mutate Docker:

```sh
.local/colossus-browser-package/colossus __browser-bundle-info
.local/colossus-browser-package/colossus __browser-bundle-verify \
  "$PWD/.local/colossus-browser-package/browser"
```

After checking the archive digest against the accepted descriptor, the
administrator explicitly loads the archive with the local engine:

```sh
/usr/bin/docker --host unix:///var/run/docker.sock image load \
  --input .local/colossus-browser-package/browser/runtime-image.tar
```

Keep the complete package layout when installing it. Ordinary CLI and worker
runtime construction discovers only `browser/` adjacent to its own canonical
executable and verifies every byte against its compiler-bound publisher manifest.
It uses only the administrator-owned `/usr/bin/docker`, the local
`unix:///var/run/docker.sock`, and the descriptor's immutable image ID; ambient
Docker contexts and runtime browser selectors are ignored. An absent engine or
preloaded image keeps browser automation unavailable. An invalid signed package
fails closed. The CLI retains the supervisor across runtime execution and awaits
process, egress, and private installation cleanup before rendering a final run or
worker-once result. Unknown cleanup causes command failure.

## Dedicated macOS host entry

The standalone workspace also has a macOS Embedded OSR entry. It derives the
fixed typed helper bundle path from its own installed application bundle, closes
all unrelated inherited descriptors before reading enrollment, and keeps CEF and
bounded AppKit dispatch on the original main thread. The Desktop application
retains its Tauri event loop. The native host never reexecutes its Rust entry as
a macOS helper: each copied helper initializes its scoped Chromium sandbox before
loading the framework.

After explicitly provisioning the pinned macOS distribution and building the
native CMake shim and helper targets, build and stage the dedicated source host:

```sh
export COLOSSUS_CEF_ROOT="$cef_source"
export COLOSSUS_CEF_NATIVE_LIB_DIR="$PWD/.local/cef-build-macos"
export CARGO_TARGET_DIR="$PWD/.local/native-host-build-macos"
cargo build --locked --manifest-path native/browser/driver/Cargo.toml
python3 -B native/browser/scripts/stage_host_macos.py \
  --cef-root "$COLOSSUS_CEF_ROOT" --native-build "$COLOSSUS_CEF_NATIVE_LIB_DIR" \
  --executable "$CARGO_TARGET_DIR/debug/colossus-native-browser-host" \
  --app "$PWD/.local/Colossus Browser Host.app" --platform macosarm64
```

The stager copies and ad-hoc signs a fresh background application, scoped-sandbox
helpers, framework and notices. It grants only the CEF development JIT/library
loading entitlements; Desktop microphone permission and dictation assets are not
included. The external consistency inventory keeps both release modes false.
The supervisor owns the private inherited channels and immutable enrollment.
macOS private CA and client-identity enrollment are explicitly unavailable until
an owned Keychain store is implemented and accepted; the default selector declines
personal identities. WindowServer-free headless deployment is also unavailable
on this entry. Cross-target Rust checks and staging tests do not replace native
macOS rendering or release-authentication acceptance. A macOS whole-host
containment owner still needs implementation; staging the host does not contain
its descendants or establish owned Keychain custody.

## Linux owned NSS provisioning

The standalone host always creates a fresh 0700 HOME beneath its verified private
profile parent before starting Chromium. Helpers inherit that HOME and dedicated
XDG directories. It does not discover, read, or modify a personal browser profile,
Keychain, Windows store, or existing NSS database. Native bootstrap may enroll
bounded owner-private CA, PKCS#12, and native password files; browser actions and the
model cannot supply paths, certificate fingerprints, or private keys. CA import
uses the same public-certificate validator as Desktop. PKCS#12 enrollment strips
all imported explicit trust before separately adding reviewed valid CA roots.
Only a native binding of one exact HTTPS origin to one public leaf DER SHA-256
fingerprint can select an eligible Chromium candidate. Missing, ambiguous, stale,
expired, CA, wrong-usage, and foreign-origin identities select no key.

Provisioning uses the reviewed [NSS tooling lock](nss-tools.lock.json): Debian 13
linux64 NSS `2:3.110-1+deb13u4` and NSPR `2:4.36-1`, with SHA-256 pins for
`certutil`, `pk12util`, and seven NSS/NSPR libraries. The host verifies those pins
and executes the verified open utility inode. An installer or builder must
explicitly install the locked OS packages first, then stage them:

```sh
python3 -B native/browser/scripts/stage_nss_tools.py \
  --destination "$PWD/.local/nss-tools-locked"
```

The stager copies existing verified artifacts into a new read-only directory. It
never downloads or installs packages. Runtime has no acquisition authority and
rejects other utility/library versions. PKCS#12 passwords stay in bounded native
memory and private temporary files passed with NSS's password-file option; they
never enter arguments, environment variables, logs, or renderer IPC. The supported
format is one nonempty printable ASCII password of at most 128 bytes, without
NUL, CR, or LF. Other Unicode and longer passwords are rejected before import
until their pinned NSS reader behavior has separate acceptance.

Unattended Chromium requires an empty NSS database password. The stored software
keys rely on the dedicated ephemeral 0700 HOME, 0600 database files, process
containment, and the operating system's disk protection; they are not encrypted
with a retained user password. Encrypted unattended tokens and hardware keys are
not implemented. Native import removes its temporary PKCS#12/password copies.
Before `CefInitialize`, the native host then pauses at private startup phase 4.
The supervisor retains the exact newly created original input inodes, detaches
and verifies each entry, removes only matching files, syncs the private directory,
and closes those retained descriptors before sending the fixed retirement ACK.
The native host separately requires the original input paths to be absent.
Missing ACK, failed sync, or unknown replacement ownership prevents Chromium
startup; unknown files and their cleanup obligation remain quarantined. The
mandatory private bootstrap retirement field makes older hosts reject this PKI
configuration. Public CA files may remain until session teardown. The owned HOME
is removed after acknowledged `CefShutdown`. Native cleanup retains its created
directory and parent descriptors, detaches the entry with NOREPLACE, and verifies
the moved directory inode before pruning it. A replacement stays quarantined;
completed cleanup cannot delete a later entry that reuses the basename. Cleanup
failure produces a categorical error. A source fixture result establishes native
CA/key-use conformance independently of release browser availability. Signed
installed components, full host egress confinement, and platform acceptance keep
their separate gates.

An offline diagnostic image can compose the staged utilities with an existing
CEF build image whose seven NSS/NSPR library hashes match the lock. The explicit
builder verifies that base image's immutable ID before and after this command,
then records and launches the resulting immutable ID:

```sh
docker build --network none --pull=false \
  --build-arg BASE_IMAGE=colossus-browser-build-cef:local \
  --file native/browser/component/Dockerfile.nss-diagnostic \
  --tag colossus-browser-nss-diagnostic:local .local/nss-tools-locked
```

This composition copies only the reviewed public utility binaries. It contains
no enrolled certificate, password, key, or personal store. The image labels keep
production acceptance false; a successful local build or TLS fixture does not
publish a supported browser component. Trusted native OCI enrollment separately
binds CA material, encrypted PKCS#12 secret handles, and exact HTTPS leaf pins to
one runtime, workspace, application, and conversation or workflow. The supervisor
stages owner-private files under its disposable control directory and passes only
private mount paths through inherited native bootstrap. A different owner is
rejected before staging, and an envelope with no enrolled origin receives no
identity or password material.

A trusted native import registry can authorize one exact workspace/application,
an explicit existing scope or future conversation/workflow consent, and a bounded
set of exact HTTPS origins. The factory resolves this consent against Core's full
open request after Core creates its runtime identity; each resulting material
lease binds the complete runtime/workspace/application/scope. Nothing is resolved
from browser tool arguments or runtime YAML. Since NSS CA trust applies throughout
one profile, any matching registration requires the **whole** immutable browser
origin envelope to fit its approved origin set before importing a CA or key.
Mixed approved/unapproved envelopes and ambiguous registrations fail closed.
An approved CA-only origin receives no client-identity capability and no staged
PKCS#12/password files when it has no reviewed identity binding.

The ignored Runtime fixture
`contained_chromium_enrolls_native_pki_for_the_actual_core_owner_after_policy`
exercises this registry through the real tool policy gateway and owned OCI
factory. Select `COLOSSUS_BROWSER_OCI_ACCEPTANCE_COMPONENT`, the immutable
NSS-pinned `COLOSSUS_BROWSER_OCI_ACCEPTANCE_IMAGE_ID`, and the trusted native
Docker executable through `COLOSSUS_BROWSER_OCI_ACCEPTANCE_DOCKER`, then run:

```sh
cargo test --locked -p colossus-runtime --lib \
  browser_tools::tests::native_pki::contained_chromium_enrolls_native_pki_for_the_actual_core_owner_after_policy \
  -- --ignored --exact
```

It checks server-observed CA/mTLS behavior, a future
conversation, alternate workflow identity, mixed-origin denial before allocation,
and a bound mTLS redirect to an unbound origin. An optional native-only
`COLOSSUS_BROWSER_OCI_PKI_ACCEPTANCE_RECEIPT` creates a new private public-evidence
file binding the executable, inventory and image. This fixture requires fresh
artifact validation and physical teardown; production, signing and broker
private-key custody remain false.

The actual Core registry fixture passed seven exact owner enrollments and six
allocated-host graceful shutdowns. Its server checks verified the selected
identity, alternate identity, untrusted CA and redirect denials; a mixed-origin
consent failure allocated no native host. All owned state and fixture scratch were
removed, and component/executable identity remained unchanged. This proves the
registered Core PKI path, without accepting renderer or broker private-key custody.

Run the server-observed native matrix as a non-root user in the compatible Linux
CEF lane, with a freshly inventoried component directory and the verified staged
utilities. Set `COLOSSUS_PKI_COMPONENT` to that native component directory:

```sh
python3 -B native/browser/scripts/run_host_pki_probe.py \
  --component "$COLOSSUS_PKI_COMPONENT" \
  --certutil "$PWD/.local/nss-tools-locked/certutil" \
  --pk12util "$PWD/.local/nss-tools-locked/pk12util" \
  --evidence "$PWD/.local/native-pki-acceptance-review"
```

The probe creates disposable CA/server/client material and an authenticated
proxy limited to its loopback fixture ports. It requires private-CA success and
the server's observed exact client certificate, then checks alternate, missing,
expired, wrong-usage, wrong-issuer, foreign-origin, hostname, and CA-chain trust
rejections. Every case owns a separate HOME/profile, retires positively owned
per-host source copies at the same phase-4 barrier, and requires acknowledged
shutdown, NSS HOME removal, and ESRCH for the owned Chromium process group after
native exit. Use an init/subreaper in the test lane so it can reap orphan helpers;
parent-process exit alone does not establish physical teardown. Its private receipt reports native TLS conformance
and owned cleanup independently; missing evidence or a failed prerequisite keeps
both false. The foreign-origin redirect case first requires the exact enrolled
client identity on the bound mTLS origin's 302 response, then requires TLS denial
without a client identity at the unbound destination. This checks that a selected
identity remains scoped after the browser follows a redirect.
The receipt embeds the exact host and component-inventory hashes, the verified
inventory and utility hashes, and checks that they remain unchanged after all
cases. These local fixture receipts do not grant publisher acceptance.

The 12-case result does not establish renderer private-key custody. CEF helpers
share the container's native user ID, so 0700 alone does not prevent a compromised
renderer from opening the NSS database. The separate OFF-default
`COLOSSUS_CEF_TEST_CUSTODY` CMake option and `native-custody-test` host feature
instrument a real renderer at `OnContextCreated`. The browser process positively
retains the owned `key4.db` inode before Chromium starts. Only the exact fixture
main frame may return a fresh native challenge bound to its committed document;
replacement documents retire that challenge. The renderer attempts an
open and immediately closes any unexpectedly returned descriptor without reading
key bytes. The renderer does not query its seccomp state: Chromium's pinned
sandbox deliberately traps `PR_GET_SECCOMP`. The trusted parent instead binds
one live renderer through a retained pidfd, stable kernel process start time,
exact executable and process group, ancestry, namespace PID, and native instance
challenge. Bounded procfs reads must show seccomp mode 2 and an additional filter
beyond the parent's baseline. Inaccessible metadata, ambiguity, replacement, a
stale document, or a crash fails the experiment. A fixed private receipt also
requires `EACCES` or `EPERM` and the same positively existing parent inode. Run
`scripts/run_renderer_custody_probe.py` with the same component/tool/evidence
arguments against a separately inventoried experiment build. Direct-open denial
still leaves broker access and full private-key custody unaccepted.

The latest opt-in native custody experiment completed server-observed mTLS 200,
page loading, acknowledged close and CEF shutdown, exact process-group reaping,
source-input retirement and NSS HOME removal. Its bounded live-receipt attempt
reached document commit and a native challenge, then reported `rejected` with
`process_not_found`: the parent could not authenticate a matching live renderer.
It produced no accepted direct-key-denial receipt. Direct-open denial, broker
custody, production and signing remain false. TLS success and clean
teardown cannot substitute for authenticated live-renderer denial evidence.

The proc attestor has a standalone non-root test, independent of CEF. From the
repository root, compile and run
`c++ -std=c++20 -O2 -Wall -Wextra -Werror native/browser/src/custody_proc_linux.cc native/browser/tests/custody_proc_linux_test.cc -o /tmp/colossus-custody-proc-test && /tmp/colossus-custody-proc-test`.
Its helper uses an additional allow-all BPF filter solely to check identity and
kernel metadata handling; passing these tests does not establish Chromium file
denial or private-key custody.

The pinned CEF public client-identity callback selects an offered native
`CefX509Certificate`; it does not accept a custom signing callback, private key,
or identity-store handle. Linux's owned NSS lookup therefore needs its actual
native TLS evidence. The existing macOS default Keychain/user trust and Windows
Current User ROOT/MY import flows retain their explicitly disclosed OS-user
scope. An application-owned Keychain or an in-memory Windows certificate store
does not by itself prove that Chromium discovers and uses its key. Fully owned
PKI on those platforms requires demonstrated native store discovery and isolation
in the dedicated host, server-observed exact-identity TLS, renderer/broker denial,
and removal of positively owned native keys. Signed installed components must
bind that acceptance to their exact browser/helper builds and OS policy. Never
substitute a certificate-error continuation or ignored TLS errors for CA trust.

The dedicated macOS host's HOME also uses retained parent/directory ownership and
exclusive retirement before descriptor-relative cleanup. A reused basename or
replaced directory remains untouched or quarantined with an unknown outcome;
cleanup does not rely on a pathname-only destructor. Unix metadata tests and a
macOS Rust cross-check do not establish macOS SDK, inherited-ACL, or kernel sandbox
acceptance.

Native file transfers use a separate private stage beneath the verified profile.
At most eight generated slot directories can release at most 32 MiB of content,
with a 4 MiB per-file admission/release ceiling. CEF download progress callbacks
cannot prove that its transient disk writer never exceeds that ceiling; a tmpfs
or separately accepted disk quota must provide the outer resource bound.
Uploads preserve only the approved bounded leaf name;
browser/model inputs never provide native paths. Downloads leave the final inode
absent until Chromium completes its write, then bind the actual nofollow private
regular inode and verify its size and metadata while reading quarantined bytes.
The stage retains the completed descriptor and partial-file siblings through
`CefShutdown`. Cancellation does not prove writer teardown. Cleanup errors retain
the stage's ownership obligation, and dropping it without acknowledged shutdown
preserves its inputs. Platform ACL and live transfer acceptance remain separate.

## Launch the macOS Desktop development preview

With the repository Rust toolchain, Node.js, Python 3.11+, CMake and Apple's
command-line development tools available, run from the repository root:

```sh
./scripts/desktop-dev --embedded-chromium-preview
```

The launcher verifies the pinned archive for the native macOS architecture,
builds the shim and all five helper variants with `USE_SANDBOX=ON`, prepares
debug CLI/sidecar executables, and builds Desktop with
`embedded-chromium-preview`. It uses `tauri build --debug --no-bundle` to embed
the renderer, then creates a fresh development app under
`.local/cef-desktop-preview.*/Colossus Chromium Preview.app`. This is a
checkout-bound development app: its debug sidecar manifest still selects the
prepared executables under `apps/desktop/src-tauri/binaries`. Keep the checkout
and those exact prepared bytes while running it. The app is ad-hoc signed,
unnotarized, and is not a distribution artifact.

The launcher opens the staged bundle through macOS LaunchServices with
`open -n -W`, which activates the Desktop window and waits for it to quit.
It explicitly forwards a selected `COLOSSUS_HOME` and, after credential-status
validation, an optional development authority selector. Native stdout and
stderr remain in `desktop.stdout.log` and `desktop.stderr.log` beside the app;
the launcher prints these paths. Use `tail -f` to follow them in another terminal.

The default offline dictation feature retains its three prepared resources under
`Contents/Resources/dictation`, Desktop's microphone usage description, and a
main-app-only audio-input entitlement.

The app's `Contents/Frameworks` contains the versioned CEF framework, including
all resources, locales, and `libcef_sandbox.dylib`, plus `Colossus Browser
Helper.app` and its Alerts, GPU, Plugin, and Renderer variants. Each helper has
its own executable name and bundle identifier. The stage signs copied dylibs,
then the framework, helpers, and outer app; it never signs or changes the
verified source cache. Development entitlements allow JIT and loading the
ad-hoc CEF library while Chromium's subprocess sandbox stays enabled. Signing
preserves Desktop's microphone usage description and gives the default offline
dictation feature its audio-input entitlement only on the main app; CEF helpers
retain their separate, narrower development entitlements. Signing
does not use `--deep`; recursive strict signature verification runs after
staging. The adjacent `Colossus Chromium Preview.app.browser-component.json`
consistency inventory retains false production modes and stays outside the
signed app.

For an already built debug Desktop or native acceptance example, the same
stager is available directly. Select a new app destination each time:

```sh
python3 -B native/browser/scripts/stage_macos.py \
  --cef-root "$COLOSSUS_CEF_ROOT" --native-build "$COLOSSUS_CEF_NATIVE_LIB_DIR" \
  --executable /absolute/path/to/debug/colossus-desktop \
  --dictation-resources "$PWD/apps/desktop/src-tauri/dictation-assets" \
  --app "$PWD/.local/review-preview/Colossus Chromium Preview.app" \
  --platform macosarm64
```

Use `macosx64` on Intel. `COLOSSUS_CEF_ROOT` must be the verified extraction
printed by `component.py fetch`; `COLOSSUS_CEF_NATIVE_LIB_DIR` is its separate
CMake build directory. The command prints the staged executable path for
diagnostics; launch the bundle with `/usr/bin/open -n -W APP` for foreground
activation. The canonical launcher also forwards its selected home and captures
native logs. A
flat executable launched through `tauri dev` cannot resolve CEF's framework
or sandbox helper layout. Use `--browser-preview` for the separate system
engine development lane.

## Launch and accept the Windows Desktop development component

Use Windows x64 with Visual Studio 2022 C++ desktop tools, the repository MSVC
Rust toolchain, Node.js, Python 3.11+, CMake and PowerShell 7. In the same
PowerShell terminal, run from the repository root:

```powershell
./scripts/desktop-chromium-preview-windows.ps1 -BuildOnly
npm --prefix apps/desktop run test:browser-chromium-native
```

Omit `-BuildOnly` to launch the development Desktop directly. The launcher pins
and verifies Windows CEF, builds its native shim with the sandbox enabled and
the dynamic MSVC runtime, prepares the debug Core/CLI sidecars, embeds the
renderer in a Rust client DLL and stages a fresh checkout-bound component.
`COLOSSUS_CEF_ROOT` and `COLOSSUS_CEF_NATIVE_LIB_DIR` are set for the current
terminal and printed for another terminal. An interactive Windows desktop is
required for foreground activation, compositor capture and OS input acceptance.

The staged `colossus-chromium-preview.exe` is the unchanged pinned CEF bootstrap.
Its adjacent client DLL exports CEF's five-argument `RunWinMain` entry. The entry
checks the bootstrap's full engine and sandbox compatibility versions, keeps the
scoped library loader alive, passes the real bootstrap sandbox context to
`CefExecuteProcess` and `CefInitialize`, and executes subprocesses before any
Tauri UI or acceptance home exists. A separate `colossus-browser-helper.exe`
bootstrap and minimal CEF-only helper DLL handle renderer/GPU/utility children,
so restricted subprocesses do not load the Desktop's Tauri, dictation or
credential dependencies. A plain Tauri executable still fails closed.
The client must delay-load `libcef.dll`; the stager checks its x64 PE DLL shape,
bootstrap export and import tables for both DLLs before publishing them.

The component contains all engine DLLs, snapshot data, resources, locales and
notices. CEF's low-privilege app-container read/execute ACL is granted only to
these distribution bytes. Session homes and credentials remain outside this
directory. Staging never overwrites a destination or changes the verified source
cache. Its final consistency inventory leaves both accepted release modes false.
The unsigned developer bootstrap and client obey CEF's signature parity rule;
installed artifacts require coordinated publisher signatures before inventory.

The native runner builds an acceptance client DLL and records bounded evidence
under `.local/chromium-acceptance-*/`. It verifies page requests, actual HWND
attachment and logical bounds, fixture colors in the Windows screen compositor,
fixed OS mouse and Unicode keyboard input, navigation/history, tab isolation,
overlay and viewport lease hiding, workspace generations, disabled production
automation and close acknowledgements. While the exact first renderer remains
live, a retained process handle identifies its executable and creation time; its
actual restricted/AppContainer token must have low integrity and must receive
`ERROR_ACCESS_DENIED` when opening an operator-private fixture under token
impersonation. Missing native evidence fails acceptance. The runner also rejects
sandbox-disable/debug-listener flags and requires no component helper processes
after acknowledged shutdown. This token/file check does not establish containment
of browser-process or network-service egress.

Failed native runs preserve their exact private home for inspection. No image-name
process-tree kill is used. Complete Windows acceptance, signed installed PKI and
process-wide browser containment remain required before production support.

## Dedicated Windows host and native presentation

Build the standalone private Windows host from a native x64 MSVC developer
PowerShell terminal:

```powershell
pwsh -File scripts/browser-native-host-windows.ps1
```

This builds the sandbox-enabled CEF bootstrap and helper, links the Rust host
client DLL with delayed `libcef.dll` loading, and stages a new pinned component.
Its fixed executable is `colossus-native-browser-host.exe`; Desktop dictation
resources are excluded. The consistency inventory keeps both release modes
false. The executable requires supervised anonymous inherited channels and cannot
be launched directly with a profile path or model-supplied endpoint.

`WindowsBrowserSupervisor` in `colossus-sandbox` owns a fresh AppContainer profile,
the full Job, exact-package WFP filters, private engine/profile bindings, bounded
proxy lease and retained native I/O workers. Its four anonymous pipe pairs carry
bootstrap, typed automation, cancellation and separately authenticated
presentation. The bootstrap derives its Windows directory from the OS API and
creates the fresh browser profile with the actual spawned process package SID.
Startup errors retain every partial allocation; acknowledged cleanup requires
full Job exit, positive loopback-exemption removal, all native I/O workers joined
and exact owned-directory removal. Runtime composition returns both
`RuntimeBrowserHost` and the independent supervisor, whose `shutdown` must be
awaited. `take_presentation` requires the full exact retained open request.

After staging, run the explicit native factory fixture from the same MSVC
developer terminal. Set `COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT` to the
new standalone host stage printed by the builder. Create a fresh owner-private
state parent with the existing native fixture helper, then select it explicitly:

```powershell
$BrowserAcceptanceParent = Join-Path $env:LOCALAPPDATA ("ColossusBrowserAcceptance-" + [Guid]::NewGuid().ToString("N"))
python -B -c 'import sys; from pathlib import Path; sys.path.insert(0, "native/browser/scripts"); from pki_fixture_private import private_directory; private_directory(Path(sys.argv[1]))' $BrowserAcceptanceParent
$env:COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT = $BrowserAcceptanceParent
cargo test --locked -p colossus-sandbox --test browser_windows_native -- --ignored --nocapture
```

Missing native prerequisites fail this fixture. A source inventory and explicit
state selector provide diagnostic authority only. The fixture requires actual
authenticated colored BGRA frames, native mouse/key/IME changes to an ordinary
field, password redaction, an acknowledged human-to-agent fence, read-only viewer
detachment with continued automation, foreign-origin denial, repeated native
close and positively removed profile/state. It also requires Low-integrity token
and package-profile label readback, real multipart upload bytes, allowed-redirect
binary and empty downloads, bounded ordered chunks and consumed-transfer denial.
These require a real writable AppContainer profile on Windows. It requires
graceful CEF shutdown and full Job/I/O/filter cleanup; forced Job exit is recorded
separately and cannot satisfy that receipt. Unknown cleanup preserves the private
state. This page-request fixture does not prove raw compromised-helper socket/DNS denial or
promote the whole-host network or release acceptance matrix.

The existing premerge `windows-desktop` lane runs this factory fixture through
`scripts/ci/browser-windows-native.ps1` as a required 45-minute check. It retains
the existing non-draft, authorized `ci:full`, successful current-head PR-gate
eligibility. This pinned sandboxed CEF check is separate from WebView2 Desktop
acceptance. The mandatory lane aggregate includes its outcome. The
`windows-owned-chromium-source-acceptance` artifact contains only a bounded log,
component inventory and false-production diagnostic report; profiles, transferred
bytes and keys are excluded. The job is wired but has not produced execution
evidence yet. Its source inventory keeps both release modes false.

Native acceptance may explicitly use the debug-only
`install_for_acceptance` constructor with a false-mode source inventory. Ordinary
installation requires independently authenticated publisher metadata and a true
accepted Desktop mode. Actual Windows SDK compilation, AppContainer/helper
package identity, WFP denial, loopback privileges, crash/full-tree cleanup and
interactive input acceptance are still required before that publication. The
NetworkIsolation API replaces an OS-global configuration list: a named gate
serializes cooperating Colossus supervisors and verifies unrelated entries, but
cannot prevent a foreign administrator tool from racing its replacement.
Standalone Windows private CA and client-identity support remains unavailable
until owned-store discovery, isolation and server-observed key use are accepted.

The portable offscreen presentation path connects the contained CEF host's real
BGRA frames to a native HWND or NSView in Desktop. Authenticated envelopes bind the
exact enrollment, session, tab, document, controller and viewport generations.
Human input uses closed native mouse, keyboard and IME messages. Read-only agent
viewers require the current nonzero controller generation and send no human
effects; detaching a viewer leaves automation alive. UI leases expire within
1.5 seconds, and hide/replacement await native-view and host-lease revocation.
The native presenter probe checks parent destruction and callback-triggered
destruction before each platform's interactive native browser acceptance run.
Desktop's native composition installs `ContainedBrowserComposition` with
authenticated runtime admission and a retained cleanup owner. It checks accepted
Embedded availability before opening an owned page. A human preview alone cannot
establish this supervisor boundary or same-page agent viewing.

Managed Desktop now composes that admission through a separate inherited native
credential. It binds the attested child, primary application grant, workspace
identity and fresh lifecycle. Worker/TUI credentials cannot use the endpoint;
each GUI command and frame rechecks the primary grant. Unix uses a short
owner-private socket namespace to accommodate macOS application state paths;
Windows uses a PID-verified named pipe. Neither endpoint stores authentication
keys or accepts executable, profile or CDP selectors.

Native Open verifies an existing application-owned conversation or creates a
canonical conversation and returns its exact ID. Human-to-agent transfer resolves
an existing run in that conversation through the normal effect gateway, obtains
the retained host's irreversible human fence, confirms the native document and
atomically grants Core ownership. The same GUI stream then acknowledges a fresh
read-only viewport. Window teardown detaches that viewer; explicit user Close
awaits full context cleanup. Caller loss retains independently owned cleanup.
These connected interfaces keep unavailable Desktop modes closed: the current
Linux package schema permits Headless only, and ordinary Windows/macOS
Embedded discovery still requires publisher acceptance and native OS evidence.

Human-to-agent handoff does not implement same-page agent-to-human takeover.
The native human input fence is irreversible for this host lifetime. Agent-viewer
detach preserves the host, but terminal cancellation and run completion close the
owned context. A new human page is a new session. Native human admission also
allows only the initial URL's exact origin, so multi-origin SSO requires additional
trusted admission functionality before those redirects can work in an owned page.

## Diagnostic workspace profiles

Temporary contexts remain the default. Linux's native `BrowserProfileStore` can
create/list/reset a bounded opaque profile for an authenticated workspace and
application. Its retained kernel lease excludes a second context and reset while
the browser is alive. It records exact CEF/Chromium/protocol versions and durable
active/dirty state. Normal acknowledged CEF shutdown permits reuse; forced reaping
requires explicit reset. Unknown cleanup and interrupted reset prohibit both reuse
and clearing. Native cache mounting never accepts a model or renderer pathname.

CEF persists a fixed request-context cache beneath its verified root cache path.
The same store root is added to Runtime filesystem/process denial independently
of caller policy, with bounded no-follow metadata and regular-inode alias checks.
Chromium child symlinks and sockets are inspected as metadata only. NSS homes and
private certificate inputs remain separately owned temporary custody.

This cache is unencrypted and its persistent bind mount bypasses the ephemeral
tmpfs ceiling. Production OCI installation therefore rejects `profile_store` until
confidential storage and a real disk quota are accepted. There is no native SDK
profile-management command or Desktop profile selector yet. The exact cookie
restart/reset/exclusive-lease fixture and its prerequisites are documented in
[owned browser conformance](../../docs/develop/testing.md#owned-browser-control-conformance).
Its actual five-host run passed cookie survival across fresh-host restart, absence
after reset and across two temporary contexts, concurrent reuse denial and
graceful shutdown of all five hosts, with the component unchanged.
That diagnostic proof cannot enable a production persistence capability.

## Native interface

[colossus_cef.h](include/colossus_cef.h) defines private native ABI version 2
(independent of the component inventory's schema version). Rebuild the native shim
and Rust client together; an older shim fails bootstrap compatibility validation.
Initialize before creating
the trusted UI, pump CEF on its UI/main thread, and wait for creation/close
acknowledgements. The event owner and policy callbacks must outlive shutdown.
Events can originate on IO threads; copy borrowed payloads and never unwind across
the C boundary. Result bytes remain private until runtime output release.

[colossus_cef_transfer.h](include/colossus_cef_transfer.h) defines a separate private
download interface. A trusted current-link action arms a generated private path
and an exact original HTTP(S) URL; the closed start operation accepts only the
cached intent identity. Page-initiated downloads remain blocked. CEF checks the
native document, original and final URL, redirects, four MiB ceiling and 30-second
deadline before releasing a terminal receipt. A cancellation request alone does
not prove that the file writer stopped: uncertain staging survives until native
shutdown. This source interface requires independent native transfer acceptance.

### Closed owned-artifact transfers

`browser.upload` accepts one fresh ordinary file-input reference and an opaque
existing artifact ID. The injected artifact port independently resolves that
application's available `RunInput` or `RunOutput` bytes, verifies the stored
length/digest, and rejects
private-key formats. Gateway pre-effect policy inspects the actual complete
bytes before any private file is created or `DOM.setFileInputFiles` is invoked.
Only a generated native stage path reaches that fixed internal method. Neither
tool arguments nor public observations contain file bytes or local paths.

`browser.download` accepts one fresh link reference. The host resolves its URL
against the current native document, checks immutable origin admission, and
starts only the cached reviewed URL. Every resource redirect is checked again.
After CEF reports physical completion, the stage binds the actual produced inode
and reads bounded stable bytes; metadata or cancellation alone cannot release a
file. Gateway post-effect policy inspects these complete bytes before the
artifact adapter creates an owner-bound `RunOutput`. Public output contains
opaque artifact metadata and a fixed safe display name.

Both transfers bind the application/workspace/run/session/tab/document and
control generation. Private authenticated chunks contain at most 64 KiB, complete
files at most four MiB, and one-shot pending custody expires in 30 seconds.
Native progress checks enforce admission and release bounds; callbacks cannot
promise an exact four-MiB transient disk writer limit. The containing profile's
ephemeral tmpfs provides the outer disk bound. Diagnostic persistent-cache binds
bypass that ceiling and require a separately accepted disk quota; persistent
production configuration is disabled. Unknown partial files and active
upload readers remain retained until acknowledged `CefShutdown` and supervised
whole-tree cleanup. Platform availability still depends on accepted containment.

The developer host fixture's `--transfers` tier requires actual multipart upload
HTTP bytes, binary and zero-byte native downloads, an allowed redirect, origin
denial, ordered chunk bounds, ignored page-suggested filenames, consumed-token
revocation and acknowledged shutdown. Run it only against a newly inventoried
component containing the transfer shim and the matching private wire protocol:

```sh
python3 native/browser/scripts/run_host_probe.py --component <owned-component> \
  --host <owned-component>/colossus-native-browser-host --screenshot --transfers
```

The fixture never publishes artifacts or grants production capabilities. Runtime
tests separately prove actual-byte pre-effect denial prevents native upload and
post-effect denial prevents download publication. Signed Windows/macOS native
transfer and full containment receipts remain independent acceptance gates.

The Desktop native adapter owns native parent extraction and opaque tab/context
registration. Guests are created hidden, cannot steal focus during navigation,
and have no application host objects or IPC. The shim denies unreviewed popups,
downloads, file pickers, OS-protocol execution, media/permission prompts, invalid
TLS certificates, and worker requests without the exact tab-owned policy. Native
certificate selection receives bounded public candidate DER certificates and an
exact HTTPS origin. The Desktop preview's **Browser certificates** panel can
**Review tab client identity** after a native identity has been imported and the
website requests it. The native dialog displays the exact origin and DER SHA-256
fingerprint, rejects CA/expired/ineligible identities, and selects no identity on
dismissal. It never automatically selects the first certificate or falls through
to an unrestricted platform chooser. Chromium can reuse the choice for that exact
origin in the temporary session; close all browser tabs to end that context.

Deferred requests have unique native IDs, expire after two minutes, and cancel on
navigation, stop, replacement, renderer crash, close, controller reload, workspace
change, or application drain. One native timer sweeps the bounded tab registry;
request replacement cannot accumulate timers. Completion rechecks the request,
controller generation, current workspace, certificate validity, and fingerprint.
CEF uses the platform private key; private keys never cross this ABI. The renderer
can request review for a tab but cannot supply an origin, fingerprint, key, or
password. Native-review implementation is separate from accepted installed mTLS
key use: that readiness remains false pending Mac and Windows acceptance.

The shim does not authorize agent effects, verify normal runtime permits, enforce
complete browser egress, or expose a renderer CDP endpoint. A native adapter may
call `colossus_cef_devtools` only after the runtime dispatch boundary is complete.
URL callbacks and disabled QUIC/non-proxied WebRTC are additional restrictions;
they do not prove policy enforcement for DNS, WebSocket, all background traffic,
or concurrent authority envelopes. No production browser driver is composed until
the complete containment and authenticated host boundary pass acceptance.

## Desktop platform gates

- macOS: main-process CEF loading uses the supported scoped dynamic loader and
  installs an `NSApplication` implementing `CefAppProtocol` before Tauri creates
  its application. A helper entry initializes its sandbox before loading CEF.
  The development stager provides typed helper identities, complete framework
  resources and ad-hoc signatures. The
  [native Desktop acceptance tier](../../docs/develop/testing.md#desktop-chromium-development-acceptance-on-macos)
  requires authorization of the actual main controller document and exercises
  actual rendering, event-loop coexistence, navigation, tabs, resizing,
  overlay occlusion, sandbox helper startup and acknowledged shutdown. Accessibility,
  certificates, installed Developer ID signatures and notarization retain their
  separate macOS acceptance requirements.
- Windows: the debug developer launcher uses the pinned bootstrap/client-DLL
  entry and its real sandbox context. Plain Tauri executables fail closed. The
  native Windows acceptance command above owns rendering, OS input, token/file
  denial and shutdown evidence; signed installed acceptance remains required.
- Linux: the fixture proves developer no-display operation, not a published CLI
  mode, release sandbox evidence, enterprise PKI, or complete egress enforcement.

The Desktop preview remains explicitly gated. Both desktop platforms must render
and automate the same actual native page in signed installed builds before the
integrated system browser is replaced by default.

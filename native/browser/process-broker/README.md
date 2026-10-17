# macOS browser process broker source

This independent native crate owns Main and delegated helper creation for the
guarded macOS development lane. Its independent creator/keeper executable,
Core factory composition and optional CMake/staging path are implemented in source.
A source-built patched CEF and actual native browser acceptance are still required.
Package discovery and the production browser factory remain disabled.

The final policy also excludes the separate certificate custody root from every
positive file grant, including stock literal paths, named predicates and file
extensions. File-extension issuance grants are removed. This explicit clamp
prevents a more-specific stock allow from overriding an appended generic deny.
Pure regressions compile all five admitted role policies, including GPU's
Scheme binding lists and network policy false sentinels. No pinned upstream
policy source or lock is altered.

After configuring the completed owned native component, source checks are:

```sh
cmake --build /approved/new/owned-native --target \
  colossus-mac-custody-policy-source-test colossus-mac-custody-roles-source-test
/approved/new/owned-native/colossus-mac-custody-policy-source-test
/approved/new/owned-native/colossus-mac-custody-roles-source-test
```

`colossus-mac-custody-role-denial-test` is an explicit operator-owned native
diagnostic. It applies the same five pinned role builders and local process/
custody transformers to disposable plain files, then requires native denial of
reads, writes, creation, rename, unlink, hard links and extension issuance with
bounded direct-child reaping. It opens no Keychain and has not been run as part
of source validation. It does not test the actual signed owned front loaders
and cannot issue the reviewed package acceptance report required by custody.

`BrowserCreator` starts native Core/Desktop/keeper EOF readers before creating
Main. It calls the fixed five-channel start-suspended Darwin launcher itself and
retains the actual child before validating signed code or contacting the keeper.
Keeper slot one belongs to Main; helper slots begin at two, with at most 64 known
children in the combined roster. The creator writes the fixed Main bootstrap on
descriptor seven and keeps that stream. Native composition receives only Main's
operation channels three through six and its separate output readers. `serve`
checks ownership again before resume, serves delegated helper requests, then
fences creation, reaps known helpers and Main, and queries the keeper's observed
roster. `CreatorCleanup` retains Main's actual-parent wait status, helper reap
count, unresolved creation count, and keeper-empty evidence separately. It never
asserts whole-browser cleanup or a graceful Chromium shutdown acknowledgement.

The helper-level `ProcessBroker` calls the existing `colossus-darwin-process`
start-suspended launcher, retains every returned `DarwinChild` before validating
its signed code, asks `KeeperServer` to capture the same kernel incarnation, and
returns a stopped PID. Chromium registers its existing one-shot Mojo receive
right and process requirement before sending `Resume`. The broker supplies fixed
native environment, role images and its own fixed descriptor-three bootstrap
socket. Standard input is `/dev/null`; output has separate pipes whose bounded
drainage remains a native composition obligation.
The browser protocol
has no executable, environment, cwd, FD, Mach-right or policy nomination. Unknown
switches and unsupported roles fail closed; ancillary launches such as Crashpad
need explicit reviewed roles and cannot fall back to ordinary spawn.

The parent keeps its normal PID-indexed Mojo rendezvous. A trusted helper loader
sets the enrolled logical browser PID from the broker-created bootstrap, so a
broker-parented helper does not mistake `getppid()` for its Mojo server. Broker
status and termination operations replace Chromium's `waitpid` assumptions;
the actual parent alone reaps children. Neither a PID census nor ASID-END event
is a domain receipt.

## Independent executable and Core composition

The `colossus-mac-browser-process-broker` executable has exactly two native modes:
`creator` and `keeper`. Its default build refuses entry. An explicit macOS Debug
build with `macos-process-entry-development` requires the completed, independently
signed Core/sidecar CDHash at build time through
`COLOSSUS_MAC_PROCESS_PARENT_CDHASH`. Both entries independently verify the live
direct parent's credentials and exact signed code before reading setup bytes.
The creator consumes fixed channels three through seven; the keeper consumes only
three. Core retains both stopped children before admission or resume.

The creator relays existing authenticated Core/browser and presentation frames
without interpreting them. It watches Core data-channel and keeper-channel EOF
through private native kqueues, with no Mach rights or audit-session joins.
Relay failure revokes its internal owner lease and wakes the creation fence.
Browser bootstrap's expected half-close does not revoke Core lifetime. Outputs
use constant-size drain buffers. Creator cleanup is a separate bounded receipt
with exact Main wait status, known-helper reaps, unknown creations, keeper-empty
acknowledgement, output drainage and channel closure. Core retains receipt bytes
across a finite missing-EOF timeout. These are known-child facts; creator
spawn-to-enrollment crashes and simultaneous creator/keeper loss still prevent
whole-domain acceptance.

`colossus-sandbox` composes this executable in the optional `owned_process` lane
of Debug `MacosBrowserAcceptanceConfig`. `stage_host_macos.py` requires
`--process-broker` with `--owned-process-development`, signs the fixed sibling
`Contents/MacOS/colossus-mac-browser-process-broker` without Chromium entitlements,
and inventories it alongside Main/helper dylibs and the patched framework.

Runtime embeds an independent public development signing key and fixed private
allocation root before the native broker is built. Sign the completed sidecar,
compile the native broker against that exact Core CDHash, stage the final package,
then sign its external descriptor. Runtime verifies fixed
`macos-browser-development.bundle.json` and `owned-process-host.json` through the
existing Ed25519 bundle verifier before constructing the actual Core factory and
private presenter. The signed descriptor contains no runtime-supplied trust key.
`production_accepted` must remain false. Keep the development signing seed outside
the entire browser allocation.

After preparing a new owner-private allocation and separate private seed
directory, initialize the independent public compiler trust:

```sh
cargo run --locked -p colossus-runtime \
  --features macos-browser-host-acceptance \
  --example macos_owned_process_package -- \
  trust "$MAC_BROWSER_ALLOCATION" "$MAC_BROWSER_SIGNING_DIRECTORY/seed"
COLOSSUS_SIDECAR_MAC_BROWSER_ACCEPTANCE_SEAL="$MAC_BROWSER_ALLOCATION/compiler-trust.json" \
  cargo build --locked -p colossus-sidecar --features macos-browser-host-acceptance
```

Sign the completed sidecar using the development signing procedure and record
its exact CDHash. Preserve those exact bytes throughout the following package:

```sh
COLOSSUS_MAC_PROCESS_PARENT_CDHASH="$MAC_BROWSER_PARENT_CDHASH" \
  cargo build --locked --manifest-path native/browser/process-broker/Cargo.toml \
  --features macos-process-entry-development
```

Use the full patched-framework build below and pass the resulting native broker
to the explicit owned-process stager. Prepare private `owned-process-host.json`
from its exact inventory/signature facts, with schema version one, purpose
`colossus-macos-owned-process-development`, `production_accepted: false`, and fields
`allocation_root`, `app_bundle`, `manifest_sha256`, `front_cdhash`, `body_cdhash`,
`policy_sha256`, `fixture_origin`, `broker_cdhash` and `helper_cdhashes` (renderer,
GPU, network, utility). Sign the descriptor after staging:

```sh
cargo run --locked -p colossus-runtime \
  --features macos-browser-host-acceptance \
  --example macos_owned_process_package -- \
  sign "$MAC_BROWSER_ALLOCATION" "$MAC_BROWSER_SIGNING_DIRECTORY/seed"
```

These commands prepare an explicit development package. Signing does not assert
native creator/helper acceptance or installed production capability.

For the explicit HTTPS fixture, also compile the fixed
`colossus-native-browser-tls-broker` entry against the same completed Core CDHash
using its `macos-https-broker-entry-development` feature and
`COLOSSUS_TLS_BROKER_PARENT_CDHASH`. Pass `--tls-broker` to the owned-process stager.
The signed descriptor's optional `tls` object contains `broker_cdhash`,
`destinations` and `public_roots` (reviewed public CA DER byte arrays). Its
`fixture_origin` must be one canonical `https://127.0.0.1:PORT` and its only numeric
destination must be that exact port/address. An HTTPS descriptor without this
binding fails closed. The ordinary stock HTTP diagnostic retains its original
resource marker and excludes both the TLS broker and public leaf-pin bootstrap.

### Prepare the actual Core HTTPS acceptance stage

After the patched framework and completed native artifact receipt exist, use
Python 3.11 or later with `cryptography`, the repository Rust toolchain, and a
new private evidence parent:

```sh
python3 -B native/browser/scripts/prepare_core_macos_owned.py \
  --cef-root=/absolute/patched/cef-distribution \
  --native-build=/absolute/completed/owned-native \
  --host-binary=/absolute/owned-driver/debug/libcolossus_native_browser_host.dylib \
  --output-root=/private/tmp/colossus-owned-core-evidence
```

This stage-only command creates independent public compiler trust, compiles the
exact Runtime lib-test executable without running it, copies it into the private
evidence directory, signs that copy, then compiles the native creator/keeper and
TLS entry against its frozen Core CDHash. Later Cargo builds cannot replace the
signed Core copy. An optional `--cargo-target-dir=/absolute/private/owned-cache`
reuses a private compiler cache without changing this separation. The preparer
stages and signs the complete owned app and signs its external descriptor.
The six-field public trust JSON and fixed fixture-input directory are embedded
using `COLOSSUS_MACOS_OWNED_ACCEPTANCE_TRUST` and
`COLOSSUS_MACOS_OWNED_ACCEPTANCE_INPUTS`. Neither contains a signing seed.

Synthetic HTTPS fixture files `fixture-ca.der`, `fixture-leaf.der`, and
`fixture-key.pk8` live in a private broker directory outside the browser
allocation. The leaf has the exact numeric loopback IP SAN and is verified
against the public CA through normal TLS. Fixture files are `0600`; they are
not system trust or a reviewed client identity. The preparer removes only its
unchanged private development signing seed after successful descriptor signing.
Failures retain allocations and unresolved material for inspection.

The emitted `owned-core-stage.json` contains the exact signed Core test binary
and one fixed ignored test command. The operator owns that separate native
launch. A passing preparation command asserts neither rendering, helper cleanup,
certificate custody, nor production readiness. The pure preparation contracts
can run without native browser or Security operations:

```sh
python3 -B -m unittest discover -s native/browser/scripts \
  -p test_prepare_core_macos_owned.py -v
```

### Collect pre-body denial evidence from the same signed component

After the Core test has finished and no creator owns its public probe markers,
prepare the controller against that stage's original app, framework, real bodies,
policy and complete component inventory:

```sh
python3 -B native/browser/scripts/prepare_owned_custody_proof.py \
  --stage-receipt=/private/evidence/owned-core-stage.json \
  --output-root=/private/tmp/colossus-owned-custody-evidence
```

This compiles and signs only the finite controller. It preserves the signed
component and creates five fixed plain markers under its already bound broker
root. The emitted private `prepared.json` records exactly Main, Renderer, GPU,
network and utility commands. Network and utility use the same generic helper
image with distinct authenticated roles. Each controller verifies the actual
body hash, requests the front's probe-only bootstrap mode, validates the denial
receipt, and reaps its direct child before any CEF initializer runs.

The operator runs those commands separately, saving each role's stdout JSON,
empty stderr and `{"exit_code":0}` in the three fixed filenames recorded for
that role. The collector launches nothing:

```sh
python3 -B native/browser/scripts/prepare_owned_custody_proof.py \
  --collect=/private/evidence/owned-custody/prepared.json
```

Collection rechecks the original component, controller, policy, real front/body
hashes and exact marker generations. Every role must prove all denials, positive
inherited-extension rejection and direct-child reaping. Only then does it emit
the unsigned `owned-process-custody-denials.json` and remove its own unchanged
plain markers. Failed or unknown results retain the markers and evidence. An
independently authenticated package descriptor must bind this report before
native certificate custody can accept it; collection alone grants no authority.
The separate no-CEF sentinel fixture remains a different diagnostic lane.

```sh
python3 -B -m unittest discover -s native/browser/scripts \
  -p test_prepare_owned_custody_proof.py -v
```

Core retains the stopped TLS child before checking its native identity and exact
code seal, then sends the private key/generation frame through authenticated fd3.
The native broker independently checks its compiled Core seal before reading it.
Core accepts only its sealed proxy port and a correctly hashed public leaf that
passes normal CA and IP-name verification. Main receives the private proxy
credential plus exact-origin public leaf pin; it receives no private CA or client
key. Cleanup requires the authenticated tracked-socket shutdown acknowledgement,
actual zero broker exit, no forced kill and completed output drains. TLS failure
does not suppress independent creator/keeper, proxy or output cleanup.

The owned temporary-profile marker has `loopback_only: true` and
`https_broker_leaf_pin_required: true`. HTTPS requires the exact broker pin; the
source-only HTTP fixture is still finite. The optional signed
`custody_denials_sha256` reserves the fixed `owned-process-custody-denials.json`
binding for independently reviewed native role-denial evidence. Its presence does
not enable private CA/client identity capabilities or accept certificate custody.

## Native integration ownership

The CEF component builder owns the locked Chromium patch and the private loader
ABI. The pinned CEF compatibility file selects
[`refs/tags/154.0.8037.98`](https://github.com/chromiumembedded/cef/blob/14c5a089e8452874cb1a556dc3fde26ed2d87723/CHROMIUM_BUILD_COMPATIBILITY.txt).
The exact launch, mapping and rendezvous behavior lives in
[`launch_mac.cc`](https://github.com/chromium/chromium/blob/154.0.8037.98/base/process/launch_mac.cc),
[`child_process_launcher_helper_mac.cc`](https://github.com/chromium/chromium/blob/154.0.8037.98/content/browser/child_process_launcher_helper_mac.cc),
[`child_process_launcher_helper_posix.cc`](https://github.com/chromium/chromium/blob/154.0.8037.98/content/browser/child_process_launcher_helper_posix.cc), and
[`mach_port_rendezvous_mac.cc`](https://github.com/chromium/chromium/blob/154.0.8037.98/base/apple/mach_port_rendezvous_mac.cc).
Stock public `OnBeforeChildProcessLaunch` changes only the command line and
cannot provide delegated process creation.

Native composition must create and authenticate the independent keeper and
browser socket pairs, verify the fixed signed role images and the body libraries,
supply unique Core/Desktop/keeper EOF lease endpoints, bound output drains, and keep keeper/control/native
credential channels out of all helper inheritance. Current code is a library,
with an explicitly enabled native executable, not a discoverable service or
launchd installer. The Core/browser bridge continues
to own policy permits and network revocation; this crate mints no effect authority.

`front_loader.c`, `fixed_policy.c`, `policy_transition.c`, `image_verify.c` and
`helper_body.mm` implement the no-exec transition. The policy
integration reads the existing sealed allocation resource and uses its actual
pinned main/helper builder. The source parser removes every positive process
creation/exec grant, including nested expressions, before adding final denies;
mixed grants and malformed source fail closed. The loader verifies the fixed
sibling Body.dylib against the creator's inventory SHA256, then applies the final
policy before `dlopen` of the body/CEF. The helper resolves the private logical-
parent install symbol only from the exact already-loaded CEF framework.
Packaging must still enforce signing, immutable custody and library validation.
The default diagnostic front→body `execve` implementation cannot be used
with the keeper: exec changes pidversion and invalidates its captured incarnation.
The explicit owned-process lane builds C-only fronts, helper body dylibs, and the
Rust Main body cdylib. Main registers its fixed creator stream before Rust closes
unrelated descriptors. Only after that bootstrap does the native platform loader
load CEF and resolve its private installer from the exact already loaded framework.
Missing actual private exports fail configuration, driver linkage and staging;
there is no stock-framework fallback.
The existing Darwin launcher establishes a new POSIX session. A session leader
cannot later leave through `setsid` or `setpgid` under
[XNU's job-control rules](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_prot.c).
An ASID change is not a ledger membership change. Kernel signaling validates
[PID plus pidversion](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_proc.c)
and ordinary signal authority; it does not use process-group membership.

## Crash and completion limits

The source retains returned children on validation, protocol, cancellation and
keeper-enrollment failures. `DeathFence` reads separate native-only Core, optional
Desktop and keeper lifetime leases. EOF, I/O failure or unexpected bytes atomically
fence creation before shutting down the browser socket, waking an idle or partial-
frame server. Creation is checked before spawn, after retention/enrollment and
before resume. Native spawn/resume and owner-loss publication share an admission
mutex: an admitted finite action finishes before loss is published, and no new
action can begin after publication. Orderly watcher retirement preserves the acknowledgement channel.
Shutdown records include reaped known children, unresolved creation attempts and
a separate positive keeper-roster-empty acknowledgement. The keeper attaches
private `EVFILT_PROC` exit observers while each signed incarnation is still
retained and stopped. It checks the kernel audit identity before and after
registration. A creator's `Reaped` message cannot remove a live member; the
keeper first requires its independently retained kernel exit event. On channel
loss it dispatches exact-incarnation termination, then waits within one
five-second deadline for the whole enrolled roster. Timeouts and failed observer
registration preserve unknown obligations. These events do not reap non-children
or establish a descendant domain. `retained_cleanup()`
remains inspectable after errors; an empty keeper roster never clears unknown
creation. Keeper-channel EOF dispatches exact-incarnation kills for already
enrolled members. Sending a signal is not exit acknowledgement.
Identity validation failures before any keeper request do not send a bogus
`Reaped` for an absent slot. A pending enrollment remains uncertain. Keeper
requests require exact sequence, allocation, slot, PID and absent wait status;
any I/O or acknowledgement failure permanently fences that endpoint and closes
it. Cleanup cannot retry an irreversible enrollment/reap or query emptiness
through a channel whose preceding operation may already have been applied.

The following are still missing and prevent any whole-helper cleanup receipt:

- Broker death between kernel creation and independent keeper enrollment can
  leave an unknown stopped front. A userspace acknowledgement cannot close this
  partial-creation window.
- Native composition must host the creator independently of Core/Desktop and
  supply correctly owned lease endpoints. `BrowserCreator` supplies Main creation
  and cleanup in source, but its deployment in that independent process has not
  passed native acceptance. Core must not spawn the main browser
  itself and then transfer only its PID: parent death during that transfer would
  create another unowned window. The creator must own main and helper creation.
- Complete native composition and owned state cleanup remain unaccepted. The
  kernel exit observer and keeper EOF primitive have passed isolated native
  process tests; this does not prove actual Chromium helper cleanup.
- Simultaneous creator/keeper loss has no accepted automatic kernel closure.
  Per-user launchd process-group cleanup cannot substitute for that property.
- The guarded no-exec CMake/staging targets now build against the full patched
  CEF framework, and the completed artifact gate verifies its private exports
  and all fixed native images. Build receipts alone do not prove native all-role
  Mach service/rights escape denial or actual runtime acceptance. Those proofs
  must use the same signed component, policy and body inventory; standalone
  sentinel results cannot substitute for them. Native output EOF and bounded
  drains must also be verified during real Chromium cleanup.

Native acceptance must inject cancellation and parent, creator and keeper death
at every phase: before spawn, after kernel creation, before retention, during
signature/keeper enrollment, after stopped-PID reply, before/after Mojo
registration, during resume, during CEF load, and while helpers are idle. Required
negative cases include fork, direct spawn, exec, session/group changes, arbitrary
FD/rights/environment nominations, replay, PID reuse, unregistered role, stale
slot and partial-create failures. No audit-port join/received-right probe belongs
in this acceptance suite. A cleanup error retains the allocation and disables
reuse; it never becomes an empty-domain claim.

## Source checks and build prerequisites

Portable checks do not spawn native children or load CEF:

```sh
cargo test --locked --offline --manifest-path native/browser/process-broker/Cargo.toml
cargo clippy --locked --offline --manifest-path native/browser/process-broker/Cargo.toml --all-targets -- -D warnings
clang++ -std=c++20 -Wall -Wextra -Werror -pthread native/browser/process-broker/client.cc native/browser/process-broker/tests/client_protocol.cc -o /private/tmp/colossus-broker-client-protocol
/private/tmp/colossus-broker-client-protocol
clang -std=c11 -Wall -Wextra -Werror -fsyntax-only native/browser/process-broker/front_loader.c
clang -std=c11 -Wall -Wextra -Werror native/browser/process-broker/policy_transition.c native/browser/process-broker/tests/policy_transition.c -o /private/tmp/colossus-policy-transition
/private/tmp/colossus-policy-transition
clang -std=c11 -Wall -Wextra -Werror native/browser/process-broker/image_verify.c native/browser/process-broker/tests/image_verify.c -o /private/tmp/colossus-image-verify
/private/tmp/colossus-image-verify
python3 native/browser/component/patches/macos-owned-process/verify.py --chromium-src /path/to/exact/pinned/src
```

Explicit macOS primitive acceptance uses only the fixed OS `true`/`sleep`
executables and ordinary private Unix sockets. It never runs an audit-session
port probe, CEF, a launchd job, or a Keychain operation:

```sh
cargo test --locked --offline -p colossus-darwin-process --test native_exit_observer -- --ignored --nocapture
cargo test --locked --offline --manifest-path native/browser/process-broker/Cargo.toml keeper::tests -- --ignored --nocapture
```

On macOS ARM64, both exit-observer cases and both keeper cases passed: normal
exit before direct-parent reaping, forced exit matching the actual parent's
wait status, keeper EOF termination with independent kernel acknowledgement,
and rejection of a false reap while the enrolled process remained alive.
These are known-incarnation primitive results, not browser acceptance.

CEF/Chromium rebuilding is separate from these source checks. The component
owner uses `verify.py --apply` to stage the locked client overlay and patch,
export the two private native-loader installation symbols, and build CEF at the
exact commit. New artifacts need new hashes, signing, inventory and acceptance;
the stock minimal archive is not compatible with delegated creation.

### Build the explicit development component

Start with the operator's licensed, initialized full Xcode and an approved
`depot_tools` checkout. Record their versions, the selected SDK version/build,
available APFS space, and separate source/output budgets before downloading.
The pinned [Mac guide](https://github.com/chromium/chromium/blob/154.0.8037.98/docs/mac_build_instructions.md)
gives approximately 30 GB for its optional shared Git cache, not an upper bound
for the checkout, dependencies or compiled output. Do not infer full-build space
readiness from that number. Use a dedicated new build directory and monitor its
disk usage against an explicit retained-space floor. Stop the owned fetch/build
on budget exhaustion and preserve its files for inspection.

This ARM64 recipe uses only the Release CEF configuration, two build jobs and
one concurrent link, with component builds, ThinLTO and debug symbols disabled.
These are initial resource limits, not measured memory/disk guarantees. The
Colossus native shim and Main body remain Debug-only. The pinned Chromium
[SDK configuration](https://github.com/chromium/chromium/blob/154.0.8037.98/build/config/mac/mac_sdk.gni)
identifies SDK 26.5/build 25F70 for official builds and supports system Xcode for
this nonofficial lane; a newer SDK still needs compilation and native acceptance.

Before any source acquisition, these read-only commands must succeed:

```sh
xcodebuild -version
xcodebuild -checkFirstLaunchStatus
xcrun --find ibtool
xcrun --sdk macosx --show-sdk-path
xcrun --sdk macosx --show-sdk-version
xcrun --sdk macosx --show-sdk-build-version
xcrun --sdk macosx metal --version
```

Recent Xcode installations may require a separate Metal Toolchain component.
If the last command reports that component missing, install it through Xcode's
component settings or `xcodebuild -downloadComponent MetalToolchain`, complete
any operator authentication, then repeat the preflight. ANGLE's actual Metal
shader compilation requires this tool; a successful first-launch check alone
does not establish it.

Use the exact pinned CEF `tools/automate/automate-git.py`, independently verified
against SHA256 `fe0c880fd2a91ac3ab4c82301f596295cecc1901e503507e36300a5b58578dcd`.
Set absolute paths without spaces for the new download root and approved depot
tools. Bootstrap the pinned checkout's Python and command wrappers without
updating that checkout. A fresh `gclient` download alone does not initialize
the Python wrapper required by `gn`:

```sh
env -u FORCE_MAC_TOOLCHAIN DEPOT_TOOLS_UPDATE=0 \
  /approved/depot_tools/ensure_bootstrap
```

Run this source-acquisition step only after the resource preflight and bootstrap:

```sh
env -u FORCE_MAC_TOOLCHAIN DEPOT_TOOLS_UPDATE=0 \
  python3 /approved/pinned/automate-git.py \
  --download-dir=/approved/new/cef-builder \
  --depot-tools-dir=/approved/depot_tools --no-depot-tools-update \
  --branch=8037 --checkout=14c5a089e8452874cb1a556dc3fde26ed2d87723 \
  --chromium-checkout=refs/tags/154.0.8037.98 --no-chromium-history \
  --arm64-build --no-build --no-distrib
```

The pinned [hermetic selector](https://github.com/chromium/chromium/blob/154.0.8037.98/build/mac/should_use_hermetic_xcode.py)
treats any nonempty `FORCE_MAC_TOOLCHAIN`, including `0`, as enabled. Its system
Xcode path also requires that `/Library/GoogleCorpSupport/` is absent. Verify
that selector returns `0` before running hooks. Do not enter the internal CIPD
or automatic administrator/license branch of `mac_toolchain.py`.

From the acquired `chromium/src`, run CEF's pinned generation hook before applying
our locked overlay. CEF's configured patches at this commit do not modify the
eight files locked by our patch verifier. Generation is repeated after the overlay
adds its sources:

```sh
git fetch --depth=1 origin tag 154.0.8037.98
env -u FORCE_MAC_TOOLCHAIN GN_OUT_CONFIGS=Release_GN_arm64 \
  GN_DEFINES='is_official_build=false is_component_build=false use_system_xcode=true use_remoteexec=false use_thin_lto=false symbol_level=0 concurrent_links=1' \
  python3 cef/tools/gclient_hook.py
python3 /absolute/Colossus/native/browser/component/patches/macos-owned-process/verify.py \
  --chromium-src=/approved/new/cef-builder/chromium/src --apply
gn gen out/Release_GN_arm64
autoninja -C out/Release_GN_arm64 -j 2 cefclient
python3 cef/tools/make_distrib.py --ninja-build --arm64-build --minimal \
  --no-symbols --no-docs --no-archive --output-dir=/approved/new/cef-distributions
```

Select the generated minimal distribution as `CEF_ROOT`, then record its exact
source and actual private exports. This command fails for stock CEF:

```sh
python3 native/browser/scripts/owned_process_component.py \
  --cef-root=/absolute/patched/cef-distribution --platform=macosarm64 \
  --record-source=/approved/new/cef-builder/chromium/src
cmake -S native/browser -B /approved/new/owned-native \
  -DCEF_ROOT=/absolute/patched/cef-distribution -DCMAKE_BUILD_TYPE=Debug \
  -DCOLOSSUS_CEF_MAC_SYNTHETIC_PROFILE_DIAGNOSTIC=ON \
  -DCOLOSSUS_CEF_DEVELOPER_NETWORK_ENVELOPE=ON \
  -DCOLOSSUS_CEF_DEVELOPER_PROCESS_BROKER=ON
cmake --build /approved/new/owned-native --parallel 2
COLOSSUS_CEF_ROOT=/absolute/patched/cef-distribution \
COLOSSUS_CEF_NATIVE_LIB_DIR=/approved/new/owned-native \
CARGO_TARGET_DIR=/approved/new/owned-driver \
  cargo build --manifest-path native/browser/driver/Cargo.toml --lib --locked --offline \
  --features macos-owned-process-development,macos-synthetic-profile-diagnostic
```

Configuration writes only source/framework metadata. The CMake `ALL` target
creates the completed native artifact receipt after the static archive, Main
front, and five helper front/body pairs are built. Driver linkage and staging
require that completed receipt and recheck every artifact digest, executable/
dylib type, and owned loader symbols. A configure-only directory, stock front,
or changed artifact cannot satisfy this gate.

Stage that fixed `libcolossus_native_browser_host.dylib` with
`stage_host_macos.py --owned-process-development`, the explicit synthetic-profile
selector and the native supervisor's fixed network-policy resource. Staging
performs development signing only. The independent creator/keeper deployment,
authenticated Core/TLS composition, output drainage and native all-role/crash
acceptance remain separate requirements; these commands do not launch a browser
or enable production discovery.

[CEF branch build instructions](https://chromiumembedded.github.io/cef/branches_and_building)
identify the required Xcode/SDK for branch 8037.
[Chromium's Mac build guide](https://chromium.googlesource.com/chromium/src/+/main/docs/mac_build_instructions.md)
requires the Mac SDK/APFS and describes a roughly 30 GB shared Git cache before
working-tree dependencies and outputs. Plan a dedicated build volume, exact
toolchain and bounded compile/link concurrency before the full source download;
this source increment installs no system service, toolchain or setting.

### Independent supervisor sentinel fixture

`python3 -B native/browser/scripts/prepare_supervisor_sentinel.py --destination /private/tmp/colossus-supervisor-fixture` compiles and development-signs an isolated fixture without launching it. Select the repository Rust toolchain in `PATH` first. Its fixed no-exec bodies load no CEF, request the four reviewed helper roles, and retain normal pre-body policy-denial admission. The ephemeral fixture signing key stays in the preparer's memory; only its public key is compiled into the controller and sacrificial Core. Core admits the exact signed controller, and the broker admits the exact signed Core.

An operator can run a reviewed preparation with `python3 -B native/browser/scripts/prepare_supervisor_sentinel.py --run-prepared /private/tmp/colossus-supervisor-fixture/prepared.json --case cancel`, `--case eof`, `--case parent-crash`, or `--case parent-crash-output-loss`. Both crash cases exit Core with status73 without running Rust destructors. The controller independently verifies the live creator and keeper as exact signed Core children and registers existing kernel exit observers before acknowledging the crash. It receives a read-only plain creator-receipt pipe; the diagnostic crash case additionally transfers the three fixed stdout/stderr read-only pipes before Core exit, with finite independent drains. The output-loss case leaves those diagnostic readers only in Core, so they disappear on its crash. Browser/helper identifiers, native control channels, observers and Mach rights are never transferred. Core reaps and non-child kernel exit observations are separate evidence fields; signal numbers are retained separately from exit codes.

Every case preserves its allocation and private bounded output logs. A missing creator receipt, unknown creation, nonempty keeper roster, output without verified EOF, forced cleanup, or expired cleanup deadline rejects acceptance. The twenty-second cleanup window is shorter than the sentinel bodies' sixty-second failsafe. This fixture covers only its retained known roster. It does not establish full CEF, creator spawn-to-enrollment crash closure, simultaneous supervisor loss, certificate custody, or production availability. Ordinary `cargo xtask check rust` runs portable contract checks and never launches these native cases.

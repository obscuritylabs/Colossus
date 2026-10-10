# Native Chromium feasibility component

This component embeds Chromium through pinned CEF. Its native-only C ABI hosts
real browser objects, isolated request contexts, and native child HWND/NSView
surfaces, and executes DevTools methods on those objects without a debugging TCP
listener. It is a developer feasibility component; release modes remain false.

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

## Native interface

[colossus_cef.h](include/colossus_cef.h) defines the ABI. Initialize before creating
the trusted UI, pump CEF on its UI/main thread, and wait for creation/close
acknowledgements. The event owner and policy callbacks must outlive shutdown.
Events can originate on IO threads; copy borrowed payloads and never unwind across
the C boundary. Result bytes remain private until runtime output release.

The Desktop native adapter owns native parent extraction and opaque tab/context
registration. Guests are created hidden, cannot steal focus during navigation,
and have no application host objects or IPC. The shim denies unreviewed popups,
downloads, file pickers, OS-protocol execution, media/permission prompts, invalid
TLS certificates, and worker requests without the exact tab-owned policy. Native
certificate selection receives bounded public candidate DER certificates and an
exact HTTPS origin; it defaults to no identity. CEF uses the platform private key;
private keys never cross this ABI.

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
- Windows: initialization requires the sandbox context provided by the supported
  CEF bootstrap/client-DLL entry. A plain Tauri executable with a null sandbox
  context fails closed. The repository has not yet completed that Desktop entry
  and installer transformation; the static shim alone does not satisfy the gate.
- Linux: the fixture proves developer no-display operation, not a published CLI
  mode, release sandbox evidence, enterprise PKI, or complete egress enforcement.

The Desktop preview remains explicitly gated. Both desktop platforms must render
and automate the same actual native page in signed installed builds before the
integrated system browser is replaced by default.

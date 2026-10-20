# Colossus Mobile

Native Tauri 2 human client for iOS and Android. It uses the existing Control Plane
HTTP API and shared `@colossus/ui` tokens, controls, conversation components, and
host platform icons. The existing Desktop application remains the macOS product.

From the repository root, run the mobile client on a local iPhone simulator:

```sh
./scripts/mobile-dev
./scripts/mobile-dev --ios --simulator "iPhone Duo"
```

The launcher uses the repository's pinned Rust toolchain, installs locked renderer
dependencies, initializes the ignored native project when needed, and runs Tauri with
live frontend updates. It selects an available iOS simulator; `--simulator` chooses
one by name. Prepare Xcode, an iOS runtime, CocoaPods, XcodeGen, libimobiledevice,
ios-deploy, and the simulator Rust target using the [local development setup](../../docs/develop/mobile.md#run-locally-on-a-mac).
Disconnect physical Apple devices before starting; the simulator launcher refuses
detected physical devices because Tauri tries them before simulator destinations.

For a browser-only renderer preview:

```sh
./scripts/mobile-dev --browser-preview
```

In either development mode, choose **Explore development preview** to browse labeled
sample data without a controller, runtime, or account. `?fixture=login` exercises login
presentation without real credentials. Neither fixture is included in production
builds. The browser preview cannot authenticate to a real controller; use the native
simulator app for that. Keep the launcher running while editing and stop it with Ctrl+C.

To check the renderer directly, run these commands from `apps/mobile`:

```sh
npm ci --ignore-scripts --legacy-peer-deps --install-links
npm run check
```

For browser tests, install Playwright Chromium and run `npm run test:browser`.
`PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` may select an already installed test Chromium.
Screenshots are written to ignored `test-results/`.

From the repository root, `cargo xtask check mobile` checks the renderer, wrapper
formatting, and native session-client tests. See the [mobile development guide](../../docs/develop/mobile.md)
for native SDK setup, supported login behavior, infrastructure, and publication.

Work includes separate conversation and task inventories, resumable native live activity,
in-app interaction responses, and capability-backed model, reasoning, Goal and Research
controls. Session views and paginated history share the existing released-data contracts.
Mobile chrome uses the shared compact typography with touch-sized input and message text.
Task details use the existing
bounded retained-event API, show released output, and link back to the associated
conversation. Cancellation is disabled until a native run has been allocated.
The browser suite covers password visibility/clearing, Large text and palettes, task
navigation, cancellation availability, and paginated output reconciliation. Sample
messages and tasks remain confined to the explicitly labeled development preview.

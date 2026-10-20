---
title: Develop Colossus Mobile
description: Build the iOS and Android Control Plane client, validate its session boundary, and prepare platform distribution.
audience: developer
type: how-to
---

# Develop Colossus Mobile

## Run locally on a Mac

From the repository root, start the iPhone simulator development loop:

```sh
./scripts/mobile-dev
```

The launcher installs the locked renderer dependencies, uses the Rust toolchain pinned
in `rust-toolchain.toml`, initializes the native iOS project when needed, and starts
Tauri on an available simulator. To select a particular simulator, pass its name:

```sh
./scripts/mobile-dev --ios --simulator "iPhone Duo"
```

Keep the terminal running while editing. Vite applies frontend updates without a fresh
native build; Tauri rebuilds native Rust changes. Stop development with Ctrl+C. Xcode 27
shows the simulated iPhone in [Device Hub](https://developer.apple.com/documentation/xcode/device-hub);
earlier Xcode versions use Simulator. The launcher targets simulators only.

Disconnect physical Apple devices before starting. The launcher checks both
`devicectl` and `ios-deploy` and stops when a physical device is detected, because Tauri
tries connected devices before simulator destinations. It also stops if it cannot
verify the device inventory; resolve the reported device check before retrying.

Mobile locks Tauri CLI 2.12.2 for Xcode 27 Device Hub compatibility. Use the launcher or
the local npm scripts so this locked CLI is selected. After a CLI update, regenerate
the ignored native project with `npm run ios:init -- --ci --skip-targets-install`
from `apps/mobile` if necessary, using the pinned toolchain setup in
[Build iOS](#build-ios).

The Mobile wrapper uses native Tauri 2.12.3, tauri-build 2.7.2, and JavaScript API 2.12.3
for the current UIKit lifecycle. SDK 27 requires a scene manifest; the tracked
`apps/mobile/src-tauri/Info.ios.plist` declares `TaoSceneDelegate` with
`UIApplicationSupportsMultipleScenes=false`. iOS uses native screen bounds. Keep the
scene manifest and persistent platform configuration in reviewed source so they survive
regeneration of the ignored `gen` directory.

Native hot reload uses `app.security.devCsp` in the tracked `tauri.conf.json`. This
development policy allows inline script elements for the React Refresh preamble and
limits the HMR WebSocket destinations to `ws://127.0.0.1:1422` and
`ws://localhost:1422`. The production `app.security.csp` retains its existing restrictions.

Choose **Explore development preview** in the app to inspect conversations, tasks, and
appearance controls with the visible **Development preview · Sample data** banner.
This mode needs no controller, connected runtime, credentials, or account. It is only
available in development builds. To exercise real sign-in and native transport, use
the simulator app with your organization's Control Plane and a local account; see
[sign-in and credential custody](#sign-in-and-credential-custody).

For a renderer preview without Xcode, run:

```sh
./scripts/mobile-dev --browser-preview
```

Open the local URL printed by Vite, then select **Explore development preview**.
The browser can exercise sample-data presentation and navigation; native controller
authentication requires the installed app. `?fixture=login` opens the development login
fixture without credentials. Preview fixtures are excluded from production builds.

### Prepare the iOS simulator once

Install Node.js 22.12 or newer, rustup, full Xcode, and Homebrew. Open Xcode once to finish
its setup, and confirm the selected developer directory with `xcode-select -p`.
Xcode Command Line Tools alone do not include the iOS development environment.
Follow the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) and install
the native project tools and the Apple Silicon simulator target:

```sh
brew install cocoapods xcodegen libimobiledevice ios-deploy
rustup target add --toolchain 1.96.0 aarch64-apple-ios-sim
rustup component add --toolchain 1.96.0 llvm-tools
```

Tauri's iOS commands check for libimobiledevice and ios-deploy and install missing
tools through Homebrew. Installing both with the other native tools keeps first launch
predictable.

The target command matches the current repository pin; use the channel in
`rust-toolchain.toml` when that pin changes. `aarch64-apple-ios-sim` is for an Apple
Silicon Mac. Intel simulator development needs `x86_64-apple-ios` instead. A Homebrew
`cargo` or `rustc` earlier on `PATH` can bypass the repository pin; the launcher selects
the pinned rustup toolchain explicitly.

The LLVM tools supply `llvm-objcopy`, which the Swift bridge uses to preserve native
plugin exports in optimized Xcode 27 builds. This also applies when
`CARGO_PROFILE_DEV_DEBUG=0` selects Swift release compilation. If a plugin was already
compiled without this component, stop development, clear its Cargo build outputs,
and restart the launcher after installation.

Install an iOS simulator runtime through **Xcode > Settings > Components**, or use
Apple's [platform download command](https://developer.apple.com/documentation/xcode/downloading-and-installing-additional-xcode-components):

```sh
xcodebuild -downloadPlatform iOS -architectureVariant arm64
xcrun simctl list runtimes
xcrun simctl list devices available
```

The `arm64` variant avoids downloading Intel runtime content on Apple Silicon. On an
Intel Mac, omit `-architectureVariant arm64` so Xcode selects a compatible variant.
Check Xcode's displayed download size and free disk space first: a runtime download alone
can exceed 7 GiB, and installation, simulator data, and Rust debug builds need additional
space. Reserve tens of GiB for a cold setup, especially when keeping separate Cargo
build trees. Use the [build cache cleanup commands](setup-testing.md#clear-local-build-caches)
only after stopping builds and preserving artifacts you need.

If no iPhone exists, create a supported model in Device Hub, then pass its displayed
name to the launcher. For iOS 27.1, choose **iPhone Duo**. Check the device types and
runtimes with `xcrun simctl list devicetypes` and `xcrun simctl list runtimes --json`.
Inspect the runtime's `supportedDeviceTypes` before choosing a model: an installed
runtime may support only particular devices. Optionally, create a named device through the CLI;
this example uses the iOS 27.1 iPhone Duo identifiers:

```sh
xcrun simctl create "Colossus iPhone" \
  com.apple.CoreSimulator.SimDeviceType.iPhone-Duo \
  com.apple.CoreSimulator.SimRuntime.iOS-27-1
./scripts/mobile-dev --ios --simulator "Colossus iPhone"
```

Use identifiers installed on your Mac if a different runtime is selected. Simulator
development does not require an App Store Connect record or distribution signing.
The generated `apps/mobile/src-tauri/gen` directory is ignored; the launcher can
regenerate it in another checkout. Keep persistent native customizations in reviewed
templates or configuration instead of depending on generated files.

## Current scope

`apps/mobile` is a Tauri 2 client for iOS and Android. It provides server selection,
local-account sign-in, projects, workspace fleet, saved conversations, Execute/Plan
submission and continuation, task inventory, live released output, detailed tool and
reasoning-summary activity, cancellation requests, and in-app approval/question
responses. Authenticated native SSE subscriptions resume from an exclusive cursor and
stop when the app is hidden, signs out, switches server, or leaves the conversation.
Retained snapshot reads reconcile on focus. Activity retains a live window of at most
1,000 events and 1 MiB; older activity is explicitly marked as omitted while new events
continue. Cancellation becomes available after allocation of a native run identity.

The composer supports capability-gated Goal and Research modes. Execute, Plan, and
Goal support per-run configured model/reasoning choices; Research keeps its own
runtime routes. Model choices require both `agent_runs.model_selection` and
`runtime.model_catalog.v1`; the catalog is fetched from the selected connection with
project Execute permission. An older connector retains host routing defaults and does
not receive unsupported catalog requests. Provider credentials and configuration stay
on the host. Earlier conversation pages load in the app. Session views show released
activity, participants, plans, sources, artifact metadata, and authorized agent inboxes.
Local context/file maps and typed Plan continuation remain unavailable through the
current Control Plane contract.
The mobile client does not host an execution runtime. Deploy the existing
[Control Plane](../admin/cloud-control-plane.md) and explicitly enroll runtimes on
execution hosts. An online controller alone cannot execute tasks. Phone suspension
does not cancel accepted runtime work. Project membership does not expand a runtime's
local grant, tools, policy, or sandbox.

## Ownership

- `apps/ui`: shared presentation, brand assets, theme tokens, controls, conversation
  components, and `HostPlatformIcon`. It imports no transport or native bridge.
- `apps/control-plane-client`: transport-neutral TypeScript response contracts shared
  by Web and Mobile, including shared status and retained-output selectors. Keep secrets
  and host state out of these contracts.
- `crates/colossus-control-plane-client`: native human-session custody, origin validation,
  fixed operation routing, bounded HTTP responses, and uncertain-outcome classification.
- `apps/mobile`: touch navigation, server/account selection, visible-page polling,
  read projections, appearance preferences, and a narrow Tauri command/channel adapter.

Promote presentation into `@colossus/ui` when another host needs it. Desktop, Web and
VS Code should consume that source instead of copying component CSS or authority logic.
Native SDKs and mobile application grants are separate from machine connector credentials.

## Sign-in and credential custody

The user enters the organization's HTTPS origin, then sees the server's configured
sign-in methods. Paths, URL credentials, queries, and fragments are rejected. Debug
builds permit HTTP only for literal loopback development fixtures; release clients
require HTTPS. Redirects are never followed. TLS verification is always enabled.

Local login uses the existing `/auth/local`, `/api/auth/config`, and `/api/me` contract.
Native requests retain the server's Origin and CSRF checks. Session cookies remain
in native memory, are never returned to the renderer, and are cleared on sign-out,
session expiry, server replacement, and process exit. The renderer clears the password
after submission. Only server address and appearance preferences use local storage.
Remote logout is best effort; losing connectivity still drops this device's custody.

OIDC is available through **Open Control Plane**, using the system browser and the
existing identity provider. That browser session does not authenticate the native app.
A native SSO handoff and OS-protected persistent sessions require a separate reviewed
contract. Do not copy browser cookies or connector bearers into renderer storage to
bridge this gap. A server with OIDC only currently requires the browser experience.

Mutation requests are attempted once. An uncertain submission retains its original
request identity for explicit reconciliation. Queue acceptance is not runtime completion;
cancellation remains a request until the runtime reports its outcome. Project permission
denials are distinguished from an expired human session through identity reconciliation.

## Check the client

From the repository root:

```sh
cargo xtask check mobile
cargo xtask check web
cargo xtask check rust
```

After shared UI changes, refresh and check Desktop and VS Code as described in the
[shared UI package](https://github.com/obscuritylabs/Colossus/tree/main/apps/ui).
Mobile changes select the existing Desktop validation path, whose surface gate includes
Mobile. The mobile workflow also exercises browser navigation at phone widths.

From `apps/mobile`:

```sh
npx playwright install chromium
npm run test:browser
```

The development-only preview contains synthetic data and is labeled. It proves renderer
behavior, not real controller authentication or native device behavior. Native client
tests exercise real loopback HTTP, redirect rejection, bounded reads, session switching,
permission/expiry distinctions, bounded streaming/reconnect/session fencing, and single-attempt mutations. Final acceptance requires
task continuation, cancellation receipts, background/resume, and physical Android/iPhone
devices.

Run the native client's controller-to-runtime acceptance separately:

```sh
cargo test --locked -p colossus-cloud-server --lib \
  local_native_client_selects_real_model_streams_resumes_and_approves
```

This test uses disposable local accounts, the real controller, an enrolled mTLS
connector, and a real runtime with a deterministic loopback provider. It verifies
configured model and reasoning selection at execution, live output before completion,
retained activity replay after reconnect, and an approval from a separate authorized
account before a file write. It requires local listener access, uses in-memory controller
storage, and does not substitute for native WebView, device suspension, physical-device,
or PostgreSQL acceptance.

## Build Android

Install the supported [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/),
including Android SDK, NDK, Java and the Rust target. From `apps/mobile`:

```sh
npm run android:init -- --ci
npm run android:dev
npm run android:build -- --aab --target aarch64 --ci
```

The generated `src-tauri/gen` project is ignored; rerun initialization on a prepared
host. Commit persistent native customizations through a reviewed template/configuration
step before depending on them. Mobile icons are derived from the existing Colossus mark.
The manual **Colossus mobile** workflow builds an unsigned ARM64 Android App Bundle.
The NDK pin is version 28 or later for current Android native-library alignment needs.
This initial artifact does not yet provide 32-bit or emulator ABIs.

Google Play distribution requires an Android upload signing key, Play Console setup,
an approved package identity, increasing version codes and store metadata. Follow
[Android signing](https://v2.tauri.app/distribute/sign/android/). An existing Windows
or Apple signing certificate does not sign an Android bundle. Keep keystores, passwords
and store service credentials outside source and generated build outputs.

## Build iOS

Use the [local launcher](#run-locally-on-a-mac) for simulator development. For a standalone
ARM64 simulator build after the one-time setup, run these commands from `apps/mobile`:

```sh
colossus_rust_bin="$(dirname "$(rustup which --toolchain 1.96.0 cargo)")"
export PATH="$colossus_rust_bin:$PATH"
export CARGO="$colossus_rust_bin/cargo"
export RUSTC="$colossus_rust_bin/rustc"
export RUSTDOC="$colossus_rust_bin/rustdoc"
export RUSTUP_TOOLCHAIN=1.96.0
npm run ios:init -- --ci --skip-targets-install
npm run ios:build -- --target aarch64-sim --debug --ci
```

This selects the pinned Rust executables for Tauri and its child builds even when a
Homebrew Rust installation is earlier on the original `PATH`. Use the toolchain channel
in `rust-toolchain.toml` if its pin changes. The manual workflow also builds an ARM64
iOS Simulator application without store signing.
Simulator artifacts cannot be installed on physical phones or submitted to the store.
Native build jobs are manual and remain unverified until run on their SDK-equipped hosts.

For TestFlight/App Store, obtain an Apple Account and Apple Developer Program membership,
register `com.obscuritylabs.colossus.mobile` under the intended team, and create the
App Store Connect app record. Configure the matching Apple Distribution certificate,
private key, Team ID, and provisioning profile. An existing Developer ID certificate
for macOS distribution or Windows Authenticode certificate cannot substitute for this.
See [Tauri iOS signing](https://v2.tauri.app/distribute/sign/ios/) and
[Apple's certificate overview](https://developer.apple.com/help/account/certificates/certificates-overview/).
Account creation and store submission are operator steps; this branch does not upload
or publish a mobile application or configure signing secrets.

## Additional infrastructure before wider release

Complete the native SSO handoff and session lifecycle, optional Keychain/Keystore custody,
push registration and APNs/FCM delivery, artifact/attachment transfer, and device
acceptance before claiming those features. Cross-host project agent
messaging remains a separate relay and relationship-grant feature; the mobile Inbox
currently shows pending task interactions. Store privacy declarations, support/privacy
URLs, review access, account-management requirements and listing assets must reflect
the final deployed product. Keep the controller and runtime capability contracts explicit.

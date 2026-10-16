---
title: Develop Colossus Mobile
description: Build the iOS and Android Control Plane client, validate its session boundary, and prepare platform distribution.
audience: developer
type: how-to
---

# Develop Colossus Mobile

## Current scope

`apps/mobile` is a Tauri 2 client for iOS and Android. It provides server selection,
local-account sign-in, projects, workspace fleet, saved conversations, Execute/Plan
submission and continuation, cancellation requests, and an inbox of pending runtime
interactions. Requests poll while the app is visible and reconcile when it regains focus.
Approvals and question responses open the exact task in the system browser's Control
Plane. The browser requires its own authenticated session and existing permissions.

The mobile client does not host an execution runtime. Deploy the existing
[Control Plane](../admin/cloud-control-plane.md) and explicitly enroll runtimes on
execution hosts. An online controller alone cannot execute tasks. Phone suspension
does not cancel accepted runtime work. Project membership does not expand a runtime's
local grant, tools, policy, or sandbox.

## Ownership

- `apps/ui`: shared presentation, brand assets, theme tokens, controls, conversation
  components, and `HostPlatformIcon`. It imports no transport or native bridge.
- `apps/control-plane-client`: transport-neutral TypeScript response contracts shared
  by Web and Mobile. Keep secrets and host state out of these contracts.
- `crates/colossus-control-plane-client`: native human-session custody, origin validation,
  fixed operation routing, bounded HTTP responses, and uncertain-outcome classification.
- `apps/mobile`: touch navigation, server/account selection, visible-page polling,
  read projections, appearance preferences, and a narrow Tauri command adapter.

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
permission/expiry distinctions and single-attempt mutations. Final acceptance requires
an isolated real controller, local login, a connected runtime, task continuation,
cancellation receipts, background/resume, and physical Android/iPhone devices.

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

Use macOS with full Xcode and the iOS Rust targets. iOS CLI commands are only available
on macOS. From `apps/mobile`:

```sh
npm run ios:init -- --ci
npm run ios:dev
npm run ios:build -- --target aarch64-sim --debug --ci
```

The manual workflow builds an ARM64 iOS Simulator application without store signing.
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
push registration and APNs/FCM delivery, artifact/attachment transfer, fuller paginated
history, and device acceptance before claiming those features. Cross-host project agent
messaging remains a separate relay and relationship-grant feature; the mobile Inbox
currently shows pending task interactions. Store privacy declarations, support/privacy
URLs, review access, account-management requirements and listing assets must reflect
the final deployed product. Keep the controller and runtime capability contracts explicit.

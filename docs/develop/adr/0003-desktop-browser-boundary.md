---
title: "ADR 0003: Desktop browser boundary"
description: Isolated platform WebView sessions for human browsing in Desktop.
audience: developer
type: concept
---

# ADR 0003: Desktop browser boundary

- Status: accepted
- Date: 2026-09-26
- Tracking: [Desktop integrated browser #205](https://github.com/obscuritylabs/Colossus/issues/205)

## Context

Desktop needs websites and local previews beside conversations. A remote document
cannot share the privileged application's native capabilities or credential store.
An iframe also cannot provide general website compatibility or native history.
Agent automation requires its own authorized effect boundary.

## Decision

Use Tauri child WebViews with a private `colossus-native-browser` adapter for
WebView2 on Windows and WKWebView on macOS. Its safe API exposes history, bounded
native metadata, temporary session sharing, and HTTP(S) system-browser handoff.
The Desktop crate continues to forbid unsafe code. The adapter's native pointers
are borrowed only inside the owning-thread callback.

The native manager owns up to eight tabs across workspaces. Each workspace gets a
separate temporary engine profile; its tabs share that profile. Selection generations
reject commands from stale workspaces. Closing the last tab ends its authenticated
session. URLs and cookies are not canonical workspace state and do not enter journals.

The local main WebView alone has browser command permissions. Capability matching
uses its exact WebView label, not its parent window. Commands additionally check the
controller document and selected workspace. Guests cannot invoke app commands, receive
workspace event broadcasts, or access the terminal and approval protocol documents.
No generic JavaScript evaluation, filesystem, process, HTTP, or WebView-control command
is added. The native test driver can evaluate synthetic pages only in an explicit
`browser-test-bridge` build, never through a production IPC command.

The trusted pane displays tabs, native URL/history state, loading, errors, and pending
popup destinations. Native children are hidden under detected app overlays, on focus
loss, controller navigation, workspace changes, and expired viewport heartbeats.
This is required because a child WebView does not obey DOM clipping or stacking.

## Navigation and permissions

Only HTTP(S) destinations without embedded credentials are accepted. App protocol
aliases and the Desktop development origin are reserved. A loopback origin must be
explicitly opened for each tab; redirects do not automatically authorize another local
origin. Normal TLS verification stays enabled. These checks are not a private-network
firewall: arbitrary website networking, DNS rebinding, and platform subresource
differences remain outside this navigation policy. Browsing uses the user's network
independently of agent tool permissions.

Automatic popups and downloads are cancelled. Popups offer an explicit new-tab action;
downloads and incompatible authentication can use the system browser. Cookies are not
transferred. Native site permission callbacks deny elevated access. WebView2 messaging,
host objects, autofill, password saving, script dialogs, context menus, browser
accelerators, and external file drops are disabled. WebKit uses a replacement delegate
and removes inherited scripts and message handlers before external navigation.

## Explicit Windows human sign-in profiles

The default owned Chromium composition is described in [ADR 0008](0008-owned-chromium-browser.md).
Its temporary-profile admission and agent session ownership remain unchanged.
Desktop can explicitly select the existing hardened Windows WebView2 adapter for
human browsing by enabling **Remember sign-ins** or setting a custom **User-Agent**.
This is a named engine choice, not an automatic fallback after Chromium failure.

The native controller obtains the workspace from an AppState selection lease and
checks its generation before reading or changing preferences. The renderer supplies
no profile path or workspace identity. Profiles use a SHA-256 workspace key beneath
the private Desktop settings root, with retained no-reparse custody and an exclusive
writer lease held through application shutdown. Every new saved-profile admission
checks 512 MiB and 32,768-entry bounds. These are admission checks, not a hard quota
on data written during an already-open WebView2 session. At most 32 saved-profile
leases remain open in one application; a restart releases inactive leases.

The native adapter verifies the actual WebView2 private/saved mode and custom
User-Agent before external navigation. Existing messaging, host-object, autofill,
permission, file-picker and navigation restrictions still apply. TLS uses normal
Windows trust. Client-certificate requests are observed categorically and retain
the native Windows identity chooser. No private key or raw cookie crosses renderer
IPC. Cookie encryption is owned by the Windows engine; site data is not claimed to
be encrypted merely because its directory is private.

Saved profiles are human-only and do not grant an agent handoff. Agent Chromium
does not open this profile, read its cookies or borrow its native environment.
Clearing data uses WebView2's fixed ALL_PROFILE browsing-data API and requires its
completion before reporting success. Clearing stays available above the admission
cap. Saving settings closes affected human views, while agent attachments retain
their separate runtime authority. A failed clear or uncertain settings write is
reported; it is not silently retried or treated as success.

Connection diagnostics expose only origin and categorical observations. Native
navigation IDs fence stale completion callbacks. HTTP status and redirect count are
reported by WebView2; an engine that does not provide them is shown as such. These
diagnostics do not claim to bypass site authentication or embedded-browser policy.

The source-only Windows `browser-acceptance --human-profile` harness exercises
separate-process sign-in persistence, workspace isolation, actual User-Agent,
clear/restart, authentication metadata and strict HTTPS/client-certificate flows
against owned loopback fixtures. Its native execution receipts are required in
addition to renderer screenshots; source and fixture success alone do not establish
installed-byte or signing acceptance.

## Platform acceptance

Normal Windows installations offer Browser in the shared right-side tool pane.
macOS still requires the `browser-preview` Cargo feature while its platform acceptance
is completed. The native acceptance driver runs on real engines against disposable
local pages, with an isolated controller profile and a generated private home.
Windows installs a fixed native file-chooser cancellation policy before navigation;
failure to install it fails tab creation. The policy does not expose a debugging port
or a general protocol command to the renderer.

| Boundary | Evidence / release gate |
| --- | --- |
| Windows history, cookies, IPC, popups, downloads, close | Native acceptance driver; each assertion must pass. |
| macOS compilation, history, cookies, delegates | Native macOS CI and on-device acceptance remain required. |
| Main view, guest focus, overlays, DPI | UI fixtures cover responsive controls; native visual review remains required on both platforms. |
| Elevated site permissions | Native geolocation probe and dialog suppression exercise the deny-all permission callback. Device-specific camera, microphone, clipboard, and notification checks remain part of platform compatibility testing. |
| Upload, print, fullscreen escape paths | Windows acceptance verifies file-picker cancellation with user activation; macOS upload callback cancels. Browser accelerators and context menus are disabled; no print or fullscreen control is offered. |
| Controller and engine failures | Controller navigation invalidates commands; heartbeat and crash state hide guests. Forced termination and recovery need native acceptance. |
| Disk cleanup | Private sessions end when their last view closes. Engine-owned cache handles may outlive close; abnormal-termination cache cleanup remains a follow-up. |

Windows and macOS pre-merge lanes lint the adapter and run the native harness. Enabling
the macOS preview by default requires acceptance on the minimum supported macOS version.
See the [test strategy](../testing.md).

## Agent automation

The platform WebView is a human browsing surface. [ADR 0008](0008-owned-chromium-browser.md)
defines the separate Chromium effect boundary, including scoped ownership, policy,
approval, audit records, cancellation and bounded release of browser observations.

## Consequences

The platform WebView adapter contains Tauri's unstable child-WebView API and requires
platform-specific acceptance. It keeps human browsing within Desktop's native
application boundary. See [Desktop browser settings](../../desktop/tools.md#browser)
for current user-facing behavior.

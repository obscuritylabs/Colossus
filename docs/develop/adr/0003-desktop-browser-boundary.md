---
title: "ADR 0003: Desktop browser boundary"
description: Isolated native browser sessions and the boundary for future agent automation.
audience: developer
type: concept
---

# ADR 0003: Desktop browser boundary

- Status: experimental; production enablement awaits native acceptance
- Date: 2026-09-26
- Tracking: [Desktop integrated browser #205](https://github.com/obscuritylabs/Colossus/issues/205)

## Context

Desktop needs websites and local previews beside conversations. A remote document
cannot share the privileged application's native capabilities or credential store.
An iframe also cannot provide general website compatibility or native history.
Future agent automation needs a separate, authorized effect boundary.

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

## Preview acceptance and remaining release gates

The `browser-preview` Cargo feature is off by default. A normal installation does not
offer the Browser control yet. This avoids treating fixture UI tests as proof of native
isolation. The native acceptance driver runs on real engines against disposable local
pages, with an isolated controller profile and a generated private home.

| Boundary | Evidence / release gate |
| --- | --- |
| Windows history, cookies, IPC, popups, downloads, close | Native acceptance driver; each assertion must pass. |
| macOS compilation, history, cookies, delegates | Native macOS CI and on-device acceptance remain required. |
| Main view, guest focus, overlays, DPI | UI fixtures cover responsive controls; native visual review remains required on both platforms. |
| Elevated site permissions | Native geolocation probe and dialog suppression; camera, microphone, clipboard, notifications need the full platform matrix before enabling. |
| Upload, print, fullscreen escape paths | macOS upload callback cancels; Windows file picker has no implemented cancellation hook. Complete this boundary before enabling. |
| Controller and engine failures | Controller navigation invalidates commands; heartbeat and crash state hide guests. Forced termination and recovery need native acceptance. |
| Disk cleanup | Private sessions end when their last view closes. Engine-owned cache handles may outlive close; verify cache removal on exit and abnormal termination before enabling. |

Windows and macOS pre-merge lanes lint the adapter and run the native harness. Also
test the minimum supported macOS version before release. Keep #205 open until these
gates and its acceptance criteria are demonstrated. See [test strategy](../testing.md).

## Future automation

Keep human browsing separate from model and tool execution. Future browser tools must
live behind a runtime port/adapter with scoped session and tab identity, policy decisions,
audit records, cancellation, bounded results, and approvals for effects where required.
They may reuse engine integration without receiving the human browser's cookies or
authority implicitly. A deliberate session-sharing design is a separate feature.

## Consequences

This avoids bundling another browser engine, but requires platform-specific acceptance
and retains Tauri's unstable child-WebView API behind one adapter. Persistent profiles,
session restoration, downloads, broad site permissions, and agent automation are
separate work. The preview does not change runtime tool authority or the domain layer.

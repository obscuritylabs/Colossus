---
title: "ADR 0007: Owned Chromium browser"
description: Shared browser authority and the boundary between Core automation and native Desktop presentation.
audience: developer
type: concept
---

# ADR 0007: Owned Chromium browser

- Status: foundation implemented; native production enablement requires acceptance
- Date: 2026-10-09
- Related decision: [Desktop browser boundary](0003-desktop-browser-boundary.md)

## Context

Colossus needs browser automation in Core and unattended deployments, with strong
Desktop support. Platform WebViews provide useful human browsing but do not provide
one portable automation engine. Desktop must let the user see the page the agent
controls. A browser extension remains a separate future attachment adapter.

Browser pages are untrusted documents. A renderer, page, model tool argument, or
possession of a session identifier cannot grant browser authority. Certificate trust
and client identities also need native provisioning: the runtime HTTP client's TLS
configuration does not configure Chromium.

## Decision

Use shared typed contracts and an application-owned `BrowserDriver` port.
`colossus-browser` owns sessions, single-writer leases, control generations, document
and snapshot references, limits, and cancellation. `colossus-runtime` derives the
application, workspace, scope, and run identity from authenticated runtime state.
Its browser adapter consumes the exact effect permit before calling the coordinator.
Browser observations require post-effect release under both built-in and external
policy. Tool availability is a separate fact from action permission.

Implement native Chromium presentation through a pinned CEF component behind
`colossus-native-browser`, preserving the Tauri shell. The native host uses private
in-process DevTools methods; it does not open a remote-debugging TCP endpoint or
expose raw CDP or arbitrary JavaScript to web IPC. The component's archive checksums,
file inventory, licensing notices, sandbox startup, and platform bootstrap are part
of its distribution contract.

Desktop's browser host and Core's sidecar require a private authenticated native
bridge before shared-page automation can be enabled. The bridge must bind one host
enrollment to the supervised runtime, workspace, application, and exact owned native
session. A renderer cannot supply an executable, native parent handle, personal
profile, transport endpoint, or execution authority.

Availability remains false until the installed engine can enforce the complete
network envelope. CEF navigation and resource callbacks alone do not contain
WebSocket, WebRTC, service-worker, and background traffic. A working native display
or a successful DevTools command is insufficient evidence for production automation.
Embedded and headless placement are accepted independently. Missing components and
unsupported bootstrap fail explicitly; there is no personal-browser fallback.

PKI provisioning and selection remain native operations. Private CA trust and client
identity import must disclose the actual OS-user or NSS-store scope. A temporary
browser profile does not imply isolated certificate custody. Identity selection
requires an exact HTTPS origin and reviewed certificate fingerprint; private keys
and import passwords never become model arguments or ordinary renderer values.
Normal server verification remains enabled.

## Consequences

Core, workflows, CLI, and Desktop can share one tool and ownership contract without
linking runtime or policy implementation into Desktop. A later MCP facade or browser
extension can implement the same port without creating another policy path.

The implementation includes a foundation and an explicit native preview boundary.
It does not change the shipping platform-WebView default until native platform
acceptance and the private bridge are complete. Engine replacement, native PKI,
unattended operation, and shared-page automation each require their own evidence.
Snapshots remain bounded text observations; screenshots, downloads, uploads,
persistent profiles, and cross-run reattachment require additional artifact or
lifecycle contracts before they are advertised.

See the [feature inventory](../feature-inventory.md) for implementation and evidence
owners. The existing [Desktop browser decision](0003-desktop-browser-boundary.md)
continues to describe the shipping human browsing surface.

---
title: "ADR 0008: Owned Chromium browser"
description: Shared browser authority and the boundary between Core automation and native Desktop presentation.
audience: developer
type: concept
---

# ADR 0008: Owned Chromium browser

- Status: shared control and native Desktop composition implemented; functional and release gaps remain
- Date: 2026-10-10
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

The managed sidecar now composes that private attachment through a separate native
credential, enrolled parent/child process identities, and the exact workspace and
application. Ordinary worker authentication cannot allocate a native human page.
The native session owns a canonical conversation; the Desktop pane can hand its
page to a run only in that conversation. It revokes human input before awaiting
the handoff and becomes read-only while the agent owns the page. A cancelled or
uncertain handoff leaves input revoked and pixels hidden. This source composition
does not advertise embedded availability without an accepted platform supervisor.

`colossus-browser-bridge` implements separate authenticated action and cancellation
channels, validates readiness on both channels, and owns a bounded per-session host
pool. The supervisor retains each allocation before attempting process startup.
Unknown allocation and teardown results remain owned obligations. Native CEF
terminal cancellation and run completion close the owned context. The coordinator
requires a new session instead of issuing a writer lease for a destroyed page.

Availability remains false until the installed engine can enforce the complete
network envelope. CEF navigation and resource callbacks alone do not contain
WebSocket, WebRTC, service-worker, and background traffic. A working native display
or a successful DevTools command is insufficient evidence for production automation.
Embedded and headless placement are accepted independently. Missing components and
unsupported bootstrap fail explicitly; there is no personal-browser fallback.

Run production agent browsing in a separately supervised browser host with one
immutable effective network envelope. The host's browser, network service, renderer,
worker, and helper processes must all inherit the operating-system restriction;
sandboxing renderers alone leaves the browser and network service able to bypass it.
The current in-process Desktop preview shares Desktop's network authority and cannot
establish this boundary. A private authenticated connection by itself does not change
that fact.

Core dispatch and native presentation are separate connections to the same owned
host. Native parent handles are process-local presentation data and never runtime or
renderer authority. In particular, a macOS `NSView` cannot be transferred to another
process to reuse the current child-view adapter. A production presentation adapter
must establish a bounded native frame or texture relay and native input ownership,
with independent compositor, keyboard, IME, focus, overlay, and accessibility
acceptance. `colossus-browser-presentation` authenticates bounded frames and binds
input to the admitted session, tab, document, control generation, and viewport
epoch. Native HWND and NSView consumers own their pixel copies and expire visible
surfaces independently of the frame stream. Connected Desktop/SDK/worker composition
is implemented in source; native platform acceptance remains required before it
replaces the preview.

Transferring a human page requires an authenticated native input-fence receipt.
The native host stops human input before reporting its current document. Core then
compares the original session, selected tab, document and control generation,
adopts that exact native document, and grants one writer in a single state change.
It clears prior references and requires a fresh snapshot. Ordinary writer grants
cannot attach to a fresh human allocation. A lost or rejected fence receipt keeps
the native page fenced and retains its cleanup owner.

Network revocation must stop accepting connections and acknowledge closure of every
active connection before reporting quiescence. An authenticated HTTP proxy is only
one part of the boundary. CONNECT destination and TLS SNI checks do not inspect
encrypted HTTP authority or prevent HTTP/2 origin coalescing. The installed host
must prove request-origin enforcement as well as direct TCP, UDP, QUIC, WebRTC,
loopback, and DNS bypass denial before advertising browser availability.

PKI provisioning and selection remain native operations. Private CA trust and client
identity import must disclose the actual OS-user or NSS-store scope. A temporary
browser profile does not imply isolated certificate custody. Identity selection
requires an exact HTTPS origin and reviewed certificate fingerprint; private keys
and import passwords never become model arguments or ordinary renderer values.
Normal server verification remains enabled.

The Linux developer host provisions a fresh owner-private NSS home and uses pinned
NSS tools. CA trust is explicit; importing a PFX never implicitly trusts its CA
chain. Client selection checks the current exact HTTPS origin, eligible native
certificate DER, validity, and reviewed leaf fingerprint. The maintained native
TLS fixture covers successful private CA and client-key use, invalid certificates,
unapproved redirects, and cleanup. The actual Core per-request registry fixture
also passed seven exact owner enrollments, six allocated-host graceful shutdowns,
server-observed identity/CA/redirect cases and mixed-origin consent rejection
before allocation, with all owned state removed. A separate corrected
renderer-custody experiment completed mTLS, page loading and shutdown. Its latest
bounded live-receipt attempt reached a native challenge, but reported `rejected`
with `process_not_found`: the parent could not authenticate a matching live
renderer. No valid direct-key-denial receipt was accepted. Linux renderer and
broker key custody therefore remain unaccepted. These TLS results also do not
establish macOS or Windows certificate-store custody,
signed installation, or whole-host containment.

CLI and the managed sidecar share fixed installed-package verification. Each
executable embeds its own publisher-verified manifest at build time; ordinary
discovery verifies the adjacent payload again before injecting a browser driver
into its runtime. The worker retains a separate supervisor and awaits shutdown
on setup, serving and error exits. Current package discovery supports Linux
headless placement only. The Windows supervisor and Desktop presentation adapters
remain development components; macOS still lacks an accepted separate-host
containment owner. These paths do not enable production Desktop attachment. No
publisher-accepted browser payload currently ships, so ordinary CLI and worker
discovery remain unavailable instead of choosing an ambient browser or downloading
one.

Temporary profiles remain the default. The Linux native store implements explicit
opaque workspace/application profiles, one retained kernel writer lease, exact
CEF/Chromium/protocol compatibility, durable active/dirty state, and reset after
acknowledged process cleanup. CEF uses a fixed per-request-context cache beneath
the retained root cache path. Runtime filesystem and process adapters protect the
same native store independently of model grants, including inode aliases.
Unknown cleanup or an interrupted reset fences reuse and cannot authorize clearing
the cache. Five actual sequential native hosts demonstrated server-observed cookie
survival across a fresh-host restart, absence after reset and between temporary
contexts, concurrent reuse rejection and graceful cleanup of every host.
This private cache is unencrypted and its persistent bind mount has no
disk quota. Ordinary OCI installation rejects this option; it is diagnostic-only
until storage protection and resource bounds are accepted. Native SDK profile
management and a Desktop profile selector are not yet implemented.

## Current enablement and remaining work

Completed source and native fixture evidence have separate scopes. A standalone
CEF host result does not prove Core's supervised OCI launch, signed installation,
or whole-host network and certificate custody. The curated OCI seccomp policy
omitted the read-only `sysinfo` call used during Chromium's pre-sandbox memory
query. A restricted startup diagnostic with the identical component completed
CEF initialization after allowing only that additional call. The source policy
includes the correction. The actual three-channel Core-to-OCI fixture then passed:
policy denied an unreviewed origin before allocation, a real page snapshot exposed
its ordinary accessibility label and redacted its protected value, and run finish
acknowledged graceful CEF shutdown and complete physical state cleanup. The
four-channel Core fixture passed authenticated 801×601 BGRA page frames, rejection
of human input/focus and invented human generation, and independent viewer detach
without losing the agent page. The five-host profile and per-request PKI fixtures
also passed with graceful physical cleanup. Each fixture checked the unchanged
component. These are actual Core conformance results, not production containment
or signed-distribution acceptance.

| Area | Implemented or demonstrated | Remaining requirement |
| --- | --- | --- |
| Shared authority and artifacts | Typed tools, one writer, stale-reference fencing, authenticated private channels, retained cleanup, screenshot/upload/download gateway release | Native action and cleanup acceptance must match the exact installed component and image |
| macOS human preview | Native rendering, AppKit coexistence and preview lifecycle acceptance | Implement separate-host whole-process containment and owned certificate custody, then accept signed native composition; preview success does not enable automation |
| Native Desktop attachment | Managed SDK/sidecar/worker admission, canonical conversation ownership, native human fence and atomic human-to-agent handoff, read-only native frames | Connected Windows/macOS OS acceptance; same-page agent-to-human takeover is absent. Terminal cancellation and run completion close the owned context; human access requires a fresh session |
| Linux contained host | Networkless OCI supervisor, authenticated egress relay, exact process identity and cleanup ownership; actual Core open/snapshot, read-only frames/detach, profile restart/reset and per-request PKI fixtures with graceful physical cleanup | Broker/socket denial, crash/reaping and installed-package acceptance |
| Windows contained host | AppContainer/Job/WFP supervisor, private I/O, pinned helper/client staging and native frame/input fixtures; source checks | Actual Windows SDK/OS execution, browser/network-service/helper denial, full-tree cleanup, native input and installed signing acceptance |
| PKI | Native exact-origin reviewed identity selection; Linux owned NSS provisioning, server-observed TLS and actual Core per-request enrollment/consent/cleanup fixtures | Linux direct-key-denial and broker custody are unaccepted; macOS separate-host custody is unsupported; Windows owned-store and broker acceptance remain incomplete. Existing OS-user stores are not isolated by a temporary browser profile |
| Persistent profiles | Linux native store, exclusive lease, compatibility/dirty/reset behavior and independent tool filesystem denial; actual five-host cookie restart/reset/temporary-isolation proof | Encryption and disk quota, native management and Desktop controls. Production configuration remains disabled |
| Human multi-origin login | Native human admission binds the initial URL's exact origin | Trusted review/admission of additional origins; cross-origin SSO redirects are currently denied in an owned human context |
| Shipping package | Shared verifier, compiler-bound publisher seal and offline Linux package builder | Independently reviewed signed component/image receipts and accepted distribution. Headless and Embedded modes stay disabled until their own acceptance passes |

These include implementation work as well as platform tests. Enabling an inventory
flag, running a preview, or passing a fixture cannot substitute for missing
capabilities or publisher acceptance. The platform WebView remains the shipping
human browsing surface, and a later extension adapter remains outside this change.

## Consequences

Core, workflows, CLI, and Desktop can share one tool and ownership contract without
linking runtime or policy implementation into Desktop. A later MCP facade or browser
extension can implement the same port without creating another policy path.

The implementation includes shared owned control and an explicit native preview boundary.
It does not change the shipping platform-WebView default until native platform
acceptance and the private bridge are complete. Engine replacement, native PKI,
unattended operation, and shared-page automation each require their own evidence.
Snapshots remain bounded text observations. Screenshots use a closed native capture
action and authenticated 64 KiB chunks; the complete bounded PNG enters the effect
gateway before an owner-bound RunOutput artifact and typed model image are published.
The next image-capable model turn receives a tool-provenance image, with the original
call association retained in the durable session. Text-only models receive the
artifact metadata; their canonical history retains the released image for later
image-capable continuation. Uploads and downloads use separate closed artifact
custody contracts rather than model-selected filesystem paths. Persistent profiles
require explicit opt-in, one native owner, compatible version metadata, and protected
storage. Native transfer, profile, and platform acceptance determine which of these
capabilities a particular installed host advertises.

See the [feature inventory](../feature-inventory.md) for implementation and evidence
owners. The existing [Desktop browser decision](0003-desktop-browser-boundary.md)
continues to describe the shipping human browsing surface.

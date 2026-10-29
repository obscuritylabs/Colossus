---
title: "ADR 0013: Human App Shots and Desktop companion"
description: Keep human foreground capture separate from agent computer authority and bind previews to an explicit chat draft.
audience: developer
type: concept
---

# ADR 0013: Human App Shots and Desktop companion

- Status: Accepted
- Date: 2026-10-29

## Context

A person needs to share an application window with a chat without granting an agent
permission to operate it. Windows capture and accessibility providers can block, and
an asynchronous capture can finish after the person switches chats or workspaces.
The optional companion needs only a short activity status, not chat contents.

## Decision

App Shots are disabled by default. Enabling them requires a native explanation and
confirmation. Only an operating-system global-hotkey callback prepares a foreground
window ticket. Renderer IPC cannot nominate a window, executable, handle or capture
operation. The native adapter freezes and revalidates an ordinary-user window identity,
and excludes protected Colossus processes. It uses target-only Windows Graphics Capture
with the platform capture indicator and bounded visible UI Automation labels; it omits
password-marked nodes and does not request full document text.

Each observation runs in an image-verified suspended child assigned to a kill-on-close
Windows Job before resume. The child verifies its actual parent’s process birth and
executable identity. Parent-side deadlines, byte/pixel bounds, process exit and a retained
Job-zero barrier precede output release. Uncertain cleanup fails closed, retains the
worker and Job in native custody, and quarantines further captures until process exit. An unavailable
accessibility provider may yield an image-only capture after acknowledged cleanup.

The native host retains one bounded capture in memory for ten minutes. Preview,
attach and discard commands belong only to the main bundled WebView. The renderer
binds the capture to its current draft and route generation; a context switch discards
it. Explicit Attach uploads selected parts through the chosen authenticated SDK as
ordinary run-input artifacts with stable idempotency keys. Normal Send is still required.
Neither capture nor attach grants agent input authority or invokes a model.

The companion is disabled by default and uses repository-owned SVG art in a separate
local WebView. That surface can read status and presentation preferences, move its own
window, reveal the main app and hide itself. It has no chat, runtime, capture or browser
IPC grants. Native code owns placement and preference persistence. Main-window status
sync exposes a closed activity enum only. System reduced motion suppresses animation.

## Consequences

Captures cannot include an elevated or protected target. Available text depends on the
app and can be absent. Pixel content may still reveal sensitive visible information;
the preview and explicit send remain the person’s review boundary. A compromised
main renderer can observe already captured previews, but cannot initiate native capture.
App Shots require Windows; this decision adds no monitor or legacy capture fallback.

See [Desktop Work](../../desktop/work.md#capture-another-windows-app) for operator-facing
behavior and [security architecture](../security-architecture.md) for agent effect authority.

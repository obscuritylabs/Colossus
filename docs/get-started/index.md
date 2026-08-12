---
title: Get started
description: Choose the shortest path from installation to a useful, policy-bounded Colossus repository run.
audience: user
type: concept
---

# Get started

You can prove that Colossus works without a network connection or model credential, then
connect a provider when you are ready. A first run uses the deterministic `echo`
provider and stores canonical state in the selected repository's isolated partition
under the owner-private Colossus home.

## The shortest path

Choose the [macOS desktop app](desktop.md) for a folder-first, zero-terminal Managed
Local setup. Choose the native interface path when you want direct CLI, TUI, daemon, or
server administration:

1. [Install the native binary](install.md).
2. [Complete the five-minute offline quickstart](quickstart.md).
3. [Connect a model](connect-model.md).
4. [Run a bounded repository task](first-repository-task.md).

## What you will have

After this journey you will have:

- one strict user-level YAML configuration, or a repository-local replacement;
- canonical state isolated by workspace beneath the Colossus home;
- a verified offline agent run;
- an optional network model route whose credential remains an environment reference;
- an explicit repository sandbox root; and
- enough context to understand approvals before allowing a mutation.

The [Colossus home reference](../reference/colossus-home.md) explains the directory
layout, configuration precedence, and automatic `AGENTS.md` instruction snapshot.

## Before you grant access

Three settings answer different questions:

- **Access** selects which tools the model can see and the default action decision.
- **Policy and approval** decide whether an exact request is allowed, denied, or needs
  your confirmation.
- **Sandbox grants** constrain the roots, executables, environment names, and network
  origins an authorized effect can actually use.

None of these settings bypasses the others. Read [Core concepts](core-concepts.md) for
the full mental model or [Access and approvals](../admin/access-and-approvals.md) before
granting broader capabilities.

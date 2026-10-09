---
title: Get started
description: See why Colossus exists, then choose the CLI or Desktop for your first run.
audience: user
type: concept
icon: lucide/rocket
---

# Get started with Colossus

Colossus helps AI agents do work beyond a chat: inspect a repository, use tools, and
carry a task across sessions. It makes their access, approvals, execution boundary,
and audit evidence visible. Start with a local run, then connect the model and
services you want to use.

## Choose your starting point

<div class="grid cards" markdown>

-   :lucide-terminal:{ .lg .middle } **CLI and terminal UI**

    ---

    Install the native command for scripts and interactive terminal work. The first
    run works offline without a model account or API key.

    [Install the CLI :lucide-arrow-right:](get-started/install.md) ·
    [Try the five-minute quickstart](get-started/quickstart.md)

-   :lucide-monitor:{ .lg .middle } **Desktop**

    ---

    Choose a folder and work in the native app. Desktop manages its own local runtime
    and includes the CLI.

    [Start on macOS :lucide-arrow-right:](get-started/desktop.md) ·
    [Start on Windows](get-started/windows-desktop.md) ·
    [Explore Desktop](desktop/index.md)

</div>

## Why Colossus exists

An agent becomes useful when it can read files, run commands, or reach a service.
Those actions also need clear authority and a record you can inspect. Colossus
brings the tools and the controls into one runtime for both connected and offline
work.

<div class="grid cards" markdown>

-   :lucide-shield-check:{ .lg .middle } **Know what it can do**

    ---

    Access selects tools. Policy and approval govern each effect. The execution
    boundary controls which host resources the effect can reach.

    [Understand the boundaries :lucide-arrow-right:](get-started/core-concepts.md)

-   :lucide-list-checks:{ .lg .middle } **Keep long work moving**

    ---

    Sessions, plans, and goals give multi-step work a durable shape. Return to a task
    after an interruption and inspect what remains.

    [Explore planning :lucide-arrow-right:](use/planning.md)

-   :lucide-file-check-2:{ .lg .middle } **Check what happened**

    ---

    Colossus records requests, decisions, effects, and uncertain outcomes in a
    verifiable journal so a restart does not silently repeat an action.

    [Explore audit and recovery :lucide-arrow-right:](admin/audit-telemetry-recovery.md)

-   :lucide-unplug:{ .lg .middle } **Work online or offline**

    ---

    Begin with a credential-free local proof, connect hosted or local models, and
    prepare controlled environments without changing the authorization path.

    [Explore offline operation :lucide-arrow-right:](admin/offline-airgap.md)

</div>

## From first run to real work

The [first run roadmap](get-started/index.md) shows the path from an offline check
to a connected model and a repository task. Read [core concepts](get-started/core-concepts.md)
before widening access. When you are ready to explore beyond the first task, see
[Use Colossus](use/index.md).

Colossus is alpha software. Review
[upgrade and compatibility guidance](get-started/upgrade-compatibility.md) before
updating an installation you depend on.

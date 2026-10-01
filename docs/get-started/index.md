---
title: First run roadmap
description: Move from an offline proof to a connected model and a first repository task.
audience: user
type: concept
icon: lucide/route
---

# First run roadmap

Start without a credential or network connection. The first run proves that the
runtime, workspace state, and audit journal work locally. Add a model route when
you are ready for a real task.

## Pick an interface

<div class="grid cards" markdown>

-   :lucide-terminal:{ .lg .middle } **CLI and terminal UI**

    ---

    Install the native binary, initialize a workspace, and run the built-in offline
    provider. Continue into an interactive terminal session or a scripted command.

    [Install the CLI :lucide-arrow-right:](install.md) ·
    [Run the quickstart](quickstart.md)

-   :lucide-monitor:{ .lg .middle } **Desktop**

    ---

    Choose a folder in the app and let Managed Local start its bundled runtime. Use
    the offline self-test before setting up a model provider.

    [macOS Desktop :lucide-arrow-right:](desktop.md) ·
    [Windows Desktop](windows-desktop.md) ·
    [Explore Desktop](../desktop/index.md)

</div>

## Move from proof to a task

1. **Verify the runtime offline.** Use the [five-minute CLI quickstart](quickstart.md)
   or Desktop's offline self-test. No model credential is needed for this check.
2. **Connect a model.** Choose a [CLI provider route](connect-model.md), or add a
   provider during Desktop setup.
3. **Explore a repository.** Run the [first repository task](first-repository-task.md)
   and check the files behind the answer.

Colossus keeps CLI and Desktop workspace state in separate partitions. The
[Colossus home reference](../reference/colossus-home.md) explains where state lives
and how configuration is selected.

## Before you grant more access

Access selects visible tools, policy and approvals govern exact requests, and the
execution boundary controls the resources an authorized effect can reach. See
[Core concepts](core-concepts.md) for how they fit together, and inspect your active
configuration before allowing file changes or process execution.

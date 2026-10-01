---
title: First repository task
description: Explore a repository with Colossus and check the files behind its answer.
audience: user
type: tutorial
icon: lucide/folder-search
---

# First repository task

With a [model connected](connect-model.md), ask Colossus to map a repository before
giving it a change to make. This first task should leave you with a short explanation
of the codebase, file paths you can inspect, and an idea for what to do next.

## 1. Open your repository

Open a terminal in the root of the repository you want to inspect, then run:

```bash
colossus models route primary
git status --short
```

The model route should name the provider and model you selected, rather than `echo`.
If it does not, follow [Connect a model](connect-model.md) for this workspace. Keep the
Git status output so you can compare it after the task; existing changes are fine.

## 2. Ask Colossus to explore

Choose the interface you prefer:

=== "Terminal UI"

    ```bash
    colossus tui
    ```

    Enter this request in the conversation:

    > Map this repository. Cite files for its main components and tests. Suggest one
    > small first task. Do not edit files.

=== "One-shot CLI"

    ```bash
    colossus run "Map this repo. Cite files and tests. Suggest a first task. No edits."
    ```

Colossus should describe the main components, point to the test entry points, and
support its findings with paths in your repository. If the answer is too broad, ask it
to inspect the cited files and name the exact functions or modules behind its claims.

## 3. Check the result

Open a few of the cited files to confirm the map. Then compare the repository status
with the output you saved before the task:

```bash
git status --short
```

The status should be unchanged because the request asked for inspection only. The
active [access and approval settings](../admin/access-and-approvals.md) and
[sandbox](../admin/sandbox.md) still govern tool authority; choose an isolated
workspace profile if you want that boundary for later tasks.

## What's next?

<div class="grid cards" markdown>

-   :lucide-play:{ .lg .middle } **Make a change**

    ---

    Give Colossus a specific edit and a test to run, then review the result.

    [Run an agent task :lucide-arrow-right:](../use/agent-runs.md)

-   :lucide-messages-square:{ .lg .middle } **Return to this work**

    ---

    Find the conversation and continue it later in the terminal UI or CLI.

    [Use sessions :lucide-arrow-right:](../use/sessions.md)

</div>

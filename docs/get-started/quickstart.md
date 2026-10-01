---
title: Five-minute quickstart
description: Run Colossus offline in a fresh folder and verify its audit journal.
audience: user
type: tutorial
icon: lucide/zap
---

# Five-minute quickstart

With the [CLI installed](install.md), you can run Colossus using its built-in `echo`
provider. This first run needs no model account, API key, or network connection.

## 1. Create a folder

=== "macOS / Linux"

    ```bash
    mkdir colossus-quickstart
    cd colossus-quickstart
    ```

=== "Windows PowerShell"

    ```powershell
    New-Item -ItemType Directory colossus-quickstart
    Set-Location colossus-quickstart
    ```

## 2. Initialize and run

=== "macOS / Linux"

    ```bash
    colossus config init --local \
      --access-profile development \
      --sandbox-profile workspace-development
    colossus run "hello from Colossus"
    ```

=== "Windows PowerShell"

    ```powershell
    colossus config init --local `
      --access-profile development `
      --sandbox-profile workspace-development
    colossus run "hello from Colossus"
    ```

The first command creates `.colossus/config.yaml` with development tools and a
workspace-isolating sandbox. Repository reads are available; file changes and shell
commands require approval. It will not overwrite an existing file. The run should
return **hello from Colossus**.

## 3. Find your configuration

```bash
colossus config effective
```

Look for `resolution.configPath` to see the active file. In this folder, edit
`.colossus/config.yaml` to change providers, access, or sandbox settings. Run
`colossus config show` when you want to see the resolved values and defaults.

## 4. Verify the audit journal

```bash
colossus audit verify
```

Verification should complete successfully. This quickstart stores its journal locally
without encryption; use [protected storage](../admin/storage-worker.md) for real data.

## What's next?

<div class="grid cards" markdown>

-   :lucide-plug:{ .lg .middle } **Connect a model**

    ---

    Use a Codex subscription, an API key, or a local model for real tasks.

    [Connect a model :lucide-arrow-right:](connect-model.md)

-   :lucide-folder-search:{ .lg .middle } **First repository task**

    ---

    Explore a repository with a connected model and check its file-backed answer.

    [Run a repository task :lucide-arrow-right:](first-repository-task.md)

</div>

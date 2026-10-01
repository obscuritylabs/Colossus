---
title: Connect a model
description: Choose a provider, set up its model route, and check that Colossus can reach it.
audience: user
type: how-to
icon: lucide/plug
---

# Connect a model

The [five-minute quickstart](quickstart.md) uses the offline `echo` provider. To use a
real model, choose the account or local server you already have. Colossus can list
available models and create a configuration for the one you select.

## 1. Open the workspace you want to use

Run setup from a repository or folder that does **not** already contain
`.colossus/config.yaml`. The `--local` command creates a configuration there and will
not overwrite one.

Already using the quickstart folder or another configured workspace? Run
`colossus config effective` to find its active file, then follow the matching
[provider guide](../use/providers/index.md) to update that configuration.

## 2. Choose a connection

Select one method. Setup loads the provider's model catalog and asks you to choose a
model by number or exact ID.

=== "Codex / ChatGPT"

    Sign in through the official Codex CLI, then choose a model available to your
    subscription:

    ```bash
    colossus codex login
    colossus provider setup --local --preset codex
    ```

    A ChatGPT/Codex subscription uses its own sign-in; it does not use an OpenAI API
    key. See [subscription setup](../use/providers/codex-chatgpt.md) for account and
    model details.

=== "OpenAI API"

    Set `OPENAI_API_KEY` in the current terminal, then run:

    ```bash
    colossus provider setup --local --preset openai
    ```

    This uses OpenAI API access and billing, which are separate from a ChatGPT
    subscription. See [OpenAI API setup](../use/providers/openai-api.md).

    ??? tip "Set an API key for this terminal"

        The key value stays out of CLI arguments and YAML.

        === "macOS / Linux"

            ```bash
            printf "OpenAI API key: "
            IFS= read -rs OPENAI_API_KEY
            printf "\n"
            export OPENAI_API_KEY
            ```

        === "Windows PowerShell"

            ```powershell
            $secret = Read-Host "OpenAI API key" -AsSecureString
            $env:OPENAI_API_KEY = [System.Net.NetworkCredential]::new("", $secret).Password
            ```

        The value lasts for the current shell session. Use your secret manager for
        persistent or unattended runs.

=== "OpenRouter"

    Set `OPENROUTER_API_KEY` in the current terminal, then run:

    ```bash
    colossus provider setup --local --preset openrouter
    ```

    Select an exact model from the catalog. See [OpenRouter setup](../use/providers/openrouter.md).

    ??? tip "Set an API key for this terminal"

        The key value stays out of CLI arguments and YAML.

        === "macOS / Linux"

            ```bash
            printf "OpenRouter API key: "
            IFS= read -rs OPENROUTER_API_KEY
            printf "\n"
            export OPENROUTER_API_KEY
            ```

        === "Windows PowerShell"

            ```powershell
            $secret = Read-Host "OpenRouter API key" -AsSecureString
            $env:OPENROUTER_API_KEY = [System.Net.NetworkCredential]::new("", $secret).Password
            ```

        The value lasts for the current shell session. Use your secret manager for
        persistent or unattended runs.

=== "Local model"

    Start your model server and load a model first. For an unauthenticated Ollama
    server on its default port:

    ```bash
    colossus provider setup --local --preset ollama --no-credential
    ```

    The connection depends on the server and selected model supporting the required
    API calls. See [local model setup](../use/providers/local-models.md) for LM Studio,
    compatibility, and manual model entry.

For Groq, Together AI, DeepSeek, Mistral, LM Studio, or a custom compatible endpoint,
run `colossus provider presets` to see the choices, then
`colossus provider setup --local` for the interactive selector. The
[provider guides](../use/providers/index.md) cover manual and existing configurations.

## 3. Check the connection

```bash
colossus models route primary
colossus models doctor
```

The route command should name the selected model and provider. `models doctor` sends
one bounded generation probe and reports `"ready": true` when the selected route
works. If the model catalog is unavailable but you know the exact model ID, add
`--model MODEL_ID` to your setup command.

For authenticated providers, the generated file contains a credential **reference**,
not the key itself. Setup also uses the standard full-access and plaintext-storage
defaults; review
`colossus config effective` and choose your [access](../admin/access-and-approvals.md),
[sandbox](../admin/sandbox.md), and [storage](../admin/storage-worker.md) settings before
giving an agent real work.

## What's next?

<div class="grid cards" markdown>

-   :lucide-play:{ .lg .middle } **Run a model turn**

    ---

    Send a bounded prompt and inspect its result.

    [Try an agent run :lucide-arrow-right:](../use/agent-runs.md)

-   :lucide-book-open:{ .lg .middle } **Provider details**

    ---

    Configure an existing workspace or troubleshoot a connection.

    [Choose a provider guide :lucide-arrow-right:](../use/providers/index.md)

</div>

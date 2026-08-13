# Colossus

Colossus is an auditable runtime for agent work and durable automation. It combines a
bounded model-and-tool loop, policy-controlled effects, resumable sessions, workflows,
memory, research, and an encrypted event journal in one Rust binary.

## Start in five minutes

Install the latest stable native binary directly from the public release channel:

```bash
curl -fsSL https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.sh | sh
```

On Windows PowerShell:

```powershell
irm https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.ps1 | iex
```

The installer selects the native archive for the host, verifies its adjacent SHA-256
sidecar, installs to `$HOME/.local` without `sudo` or profile changes, and prepares an
empty owner-private `$HOME/.colossus` home. The
[installation guide](docs/get-started/install.md) includes a review-before-running form,
exact-version selection, Nix, offline archives, supported targets, updates, and
uninstallation.

Initialize your user configuration and prove the runtime offline in the current
repository:

```bash
colossus config init
colossus run "Reply with exactly: ready"
colossus audit verify
```

The generated `echo` provider is credential-free and makes this first run completely
offline. State is isolated beneath the current workspace's partition in the Colossus
home. Start the terminal UI with:

```bash
colossus
```

The working directory, or explicit `-w`, identifies the repository Colossus can reason
about; it does not relocate the Colossus home. A repository can replace user defaults
with `.colossus/config.yaml` and can supply bounded instructions through `AGENTS.md`.
Policy, approvals, and sandbox grants still determine which effects are possible. See
[Colossus home and workspace resolution](docs/reference/colossus-home.md) for the exact
load order and storage layout.

[Read the five-minute quickstart](docs/get-started/quickstart.md) or open the
[published documentation](https://obscuritylabs.github.io/Colossus/).

Direct installations can later check or apply stable updates with:

```bash
colossus update check
colossus update
```

## What you can do

- Run bounded agent tasks with streaming, durable sessions, explicit context controls,
  and provider role routing.
- Use filesystem, Git, process, search, research, memory, integration, MCP, workflow,
  goal, and subagent capabilities through one effect gateway.
- Automate repeatable work with validated YAML workflows, schedules, webhooks,
  recovery, signed packs, and collections.
- Build Rust and Tauri applications in process, or connect enrolled Rust, TypeScript,
  Python, and Go backends through the durable authenticated application API.
- Apply access profiles, approvals, OPA policy, sandbox limits, encrypted journaling,
  audit verification, and offline operation without hiding security decisions in a UI.

## Documentation

- [Get started](docs/get-started/index.md) — install, connect a model, and complete a
  first repository task.
- [Use Colossus](docs/use/index.md) — sessions, the terminal UI, durable work, goals,
  memories, and research.
- [Automate and extend](docs/extend/index.md) — workflows, skills, integrations, MCP,
  packs, and registries.
- [Administer and secure](docs/admin/index.md) — configuration, routing, access,
  policy, storage, audit, offline operation, and troubleshooting.
- [Reference](docs/reference/index.md) — CLI, TUI, configuration, schemas, manifests,
  limits, and glossary.
- [Develop](docs/develop/index.md) — source setup, architecture, security boundaries,
  the [public application SDK](docs/develop/application-sdk.md), test tiers, and
  documentation authoring.

Release history lives in [CHANGELOG.md](CHANGELOG.md). Report vulnerabilities using
the private process in [SECURITY.md](SECURITY.md).

## Develop

Contributor setup, the Rust toolchain contract, focused test tiers, and completion
gates live in [Develop Colossus](docs/develop/index.md). Keeping source-build commands
there lets the installation and first-run paths stay binary-first.

Build or preview the documentation through the pinned containerized toolchain:

```bash
./scripts/docs-site build
./scripts/docs-site serve
```

The project is licensed under [Apache License 2.0](LICENSE).

# Colossus

**AI agent operations for real work in mission environments.**

Colossus is an alpha-stage runtime for organizations that need agents to act under
explicit authority, inside enforceable boundaries, with durable evidence of what
happened. It brings models, tools, policy, approvals, sandboxing, state, and audit into
one system that can operate online or offline.

The project is being built for serious enterprise and public-sector deployment,
including regulated, disconnected, and security-sensitive environments. It ships as a
Rust binary with a CLI and terminal UI; a Tauri desktop app and authenticated SDKs use
the same runtime and security model.

> [!WARNING]
> Colossus is alpha software. Behavior, configuration, storage formats, APIs, and user
> interfaces may change between releases. Review the [upgrade and compatibility
> guide](docs/get-started/upgrade-compatibility.md) before updating an installation you
> depend on.

## Try it offline

Install the latest stable release on macOS or Linux:

```bash
curl -fsSL https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.sh | sh
```

On Windows PowerShell:

```powershell
irm https://github.com/obscuritylabs/Colossus/releases/latest/download/colossus-install.ps1 | iex
```

If you prefer to inspect the installer first, need an exact version, or use Nix, start
with the [installation guide](docs/get-started/install.md).

Then initialize Colossus in a repository and run the built-in offline provider:

```bash
cd your-repository
colossus config init
colossus run "Reply with exactly: ready"
colossus audit verify
```

That first run needs no API key or network connection. It creates workspace-isolated
state under your owner-private Colossus home and verifies the journal that recorded the
run. Launch the terminal UI with:

```bash
colossus
```

See [Colossus home and workspace resolution](docs/reference/colossus-home.md) for the
configuration load order, state layout, and repository instruction boundary.

> [!IMPORTANT]
> Fresh schema-version-3 configurations currently default to `allow_all` with the
> acknowledged `danger_full_access` execution boundary. Authorized tools can therefore
> reach host resources outside the selected workspace. If that is not appropriate,
> initialize with `--sandbox-profile workspace-development` or
> `--sandbox-profile offline-default`. Read [Access and
> approvals](docs/admin/access-and-approvals.md) and
> [Sandbox](docs/admin/sandbox.md) before granting an agent broader authority.

Continue with the [five-minute quickstart](docs/get-started/quickstart.md),
[connect a model](docs/get-started/connect-model.md), or choose the
[Desktop setup](docs/get-started/desktop.md).

Direct installations can check for or apply a stable update with:

```bash
colossus update check
colossus update
```

## What Colossus is built for

- **Accountable effects.** Requested actions, decisions, approvals, execution, release,
  and uncertain outcomes produce durable evidence.
- **Bounded execution.** Strict tool schemas, one-use permits, sandbox profiles,
  resource ceilings, and quarantined output keep authority explicit.
- **Durable operations.** Sessions, plans, goals, delegated agents, workflows,
  memories, and research survive restarts and can be reconstructed from canonical
  state.
- **Online or offline deployment.** Connect hosted providers, run local models, or
  prepare controlled and air-gapped environments without changing the authorization
  path.
- **Enterprise integration.** Add application clients, search routes, integrations,
  standalone MCP servers, OCI-distributed Agent Plugins, and versioned YAML workflows through declared
  boundaries.

## Choose an interface

| If you want to… | Start here |
| --- | --- |
| Run one task or produce machine-readable output | [Agent runs](docs/use/agent-runs.md) |
| Work interactively in a terminal | [Terminal UI](docs/use/terminal-ui.md) |
| Use a folder-first native app | [Desktop](docs/get-started/desktop.md) |
| Build durable automation | [Workflows](docs/extend/workflows/first-workflow.md) |
| Integrate another application | [Application SDK](docs/develop/application-sdk.md) |
| Understand the trust boundaries | [Security architecture](docs/develop/security-architecture.md) |

## Documentation

- [Get started](docs/get-started/index.md): installation, first run, and model setup
- [Use Colossus](docs/use/index.md): sessions, plans, goals, memory, and research
- [Automate and extend](docs/extend/index.md): workflows, Agent Plugins, integrations,
  standalone MCP, OCI registries, and supply-chain trust
- [Administer and secure](docs/admin/index.md): configuration, access, sandboxing,
  storage, audit, and troubleshooting
- [Reference](docs/reference/index.md): CLI, TUI, schemas, formats, and limits
- [Develop](docs/develop/index.md): architecture, source setup, testing, and releases

The complete documentation is also available at
[obscuritylabs.github.io/Colossus](https://obscuritylabs.github.io/Colossus/).

## Develop

Colossus uses Rust 1.96 and edition 2024. From a source checkout:

```bash
cargo build --workspace
cargo xtask dev
```

Run `cargo xtask check rust` before declaring a Rust change complete. The
[contributor guide](docs/develop/contributing.md) explains the architecture rules,
focused test tiers, Desktop checks, and pre-PR gate.

Build or preview the documentation with its pinned containerized toolchain:

```bash
./scripts/docs-site build
./scripts/docs-site serve
```

See the [changelog](CHANGELOG.md) for release history. Report vulnerabilities through
the private process in [SECURITY.md](SECURITY.md).

Colossus is licensed under the [Apache License 2.0](LICENSE).

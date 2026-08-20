# Colossus

**A runtime for agents that do real work—and leave an audit trail.**

Colossus gives AI agents a controlled way to use files, Git, processes, network
services, and other tools. Every effect passes through access checks, policy, optional
approval, execution limits, and durable audit. Sessions, plans, workflows, research,
and other work survive restarts instead of disappearing with the terminal transcript.

It ships as a Rust binary with a CLI and terminal UI. A Tauri desktop app and
authenticated SDKs use the same runtime and security model.

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

> [!IMPORTANT]
> Fresh schema-version-2 configurations currently default to `allow_all` with the
> acknowledged `danger_full_access` execution boundary. Authorized tools can therefore
> reach host resources outside the selected workspace. If that is not appropriate,
> initialize with `--sandbox-profile workspace-development` or
> `--sandbox-profile offline-default`. Read [Access and
> approvals](docs/admin/access-and-approvals.md) and
> [Sandbox](docs/admin/sandbox.md) before granting an agent broader authority.

Continue with the [five-minute quickstart](docs/get-started/quickstart.md),
[connect a model](docs/get-started/connect-model.md), or choose the
[Desktop setup](docs/get-started/desktop.md).

## What Colossus provides

- **Controlled execution.** Strict tool schemas, access profiles, policy decisions,
  approvals, one-use permits, sandbox boundaries, and quarantined output share one
  effect path.
- **Durable work.** Sessions, tasks, decisions, plans, goals, delegated agents,
  memories, research, and workflows are event-sourced and resumable.
- **Verifiable history.** The canonical journal uses payload hashes and a global hash
  chain; protected storage can add authenticated encryption and signed checkpoints.
- **Practical extension points.** Add providers, search routes, integrations, MCP
  servers, skills, signed packs, and YAML workflows without creating a second path
  around policy.
- **Application APIs.** Embed the Rust SDK or connect TypeScript, Python, and Go clients
  through the authenticated, loopback gRPC API.

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
- [Automate and extend](docs/extend/index.md): workflows, skills, integrations, MCP,
  packs, and registries
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

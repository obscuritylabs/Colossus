# Getting Started

This guide starts the Rust runtime with fresh YAML and fresh encrypted state. The default
`echo` provider is deterministic and needs no model credential or network access.

## Install

From a native release archive, run the included clean-prefix installer:

```bash
tar -xzf colossus-VERSION-TARGET.tar.gz
./colossus-VERSION-TARGET/install.sh
export PATH="$HOME/.local/bin:$PATH"
```

On Windows:

```powershell
Expand-Archive colossus-VERSION-TARGET.zip
.\colossus-VERSION-TARGET\install.ps1
$env:PATH = "$HOME\.local\bin;$env:PATH"
```

The archive installer requires neither Cargo nor Python and makes no network request.
Verify the archive checksum—and a signed bundle when publisher authenticity is
required—before installation.

For source development, Rust 1.96 is required:

```bash
cargo run --offline \
  -p colossus-cli --bin colossus -- --version
```

To run changing debug binaries without repeated macOS Keychain prompts, use the isolated
development launcher. It clones the existing config's non-storage settings when present,
but uses separate environment keys, state, and secure anchor:

```bash
./scripts/colossus-dev --approval-mode full-access tui
```

This development state is intentionally separate from `.colossus/config.yaml` and
`.colossus/state.redb`; deleting or swapping one config does not migrate the other.

## Initialize And Smoke Test

Create strict configuration beside the fresh Rust state:

```bash
colossus --config .colossus/config.yaml config init
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml run "hello"
colossus --config .colossus/config.yaml audit verify
```

On a terminal, the run result is a Markdown-capable human card. When redirected, the
same command emits JSON containing `run_id`, `session_id`, `profile: "echo"`,
`output: "hello"`, event count, and elapsed time. Use `--output human` or
`--output json` to override automatic selection. `config init` uses the OS credential
store for mandatory journal/signing keys; headless deployments can explicitly configure
environment key references instead. Rust never silently writes plaintext canonical
state.

## Start The Terminal UI

```bash
colossus --config .colossus/config.yaml
# Equivalent explicit form:
colossus --config .colossus/config.yaml tui
```

Useful first commands:

```text
/help
/tools
/work
/context status
/exit
```

The Ratatui interface restores durable session messages into a scrollable transcript and
keeps the composer pinned at the bottom while model and tool work continues. It supports
encrypted history, streamed assistant/tool events, multiline input, themes, workflows,
goals, research, memories, slash and `@skill` completion, semantic cards, Markdown,
approval/input overlays, queued turns, and authenticated-worker routing. The removed
`colossus repl` alias is no longer part of the public CLI.

## Choose A Workspace

The process working directory is the initial workspace. Start Colossus from the
repository you want to operate on, while passing an absolute config path:

```bash
cd ../my-project
colossus --config /absolute/path/to/.colossus/config.yaml
```

Restart the TUI from another directory to change the active repository scope.
Filesystem, Git, patch, repository-context, and process effects still require matching
absolute filesystem/executable grants in YAML; changing the process workspace never
expands policy.

## Understand Approvals

One-shot commands default to `deny`; the interactive TUI defaults to `ask`. Approval modes
satisfy policy obligations but never add actions, filesystem roots, executable identities,
or network origins:

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  run 'Create note.txt with filesystem.write'
colossus --config .colossus/config.yaml --approval-mode risk-auto \
  run 'Run the configured test executable'
colossus --config .colossus/config.yaml --approval-mode full-access \
  run 'Apply the approved change'
```

`risk-auto` auto-proves only a strict low-risk/allow assessment for approval-required
`shell.run`; every other result or evaluator failure falls back to an explicit prompt.
`full-access` auto-proves only approval-required requests that policy already permits.

## Configure A Real Model

Edit the generated YAML to add either `open_ai_responses` or
`open_ai_compatible`, route `primary` to that profile, and add the exact provider
origin under `sandbox.networkDestinations`. Credentials remain references:

```yaml
providers:
  profiles:
    openrouter:
      kind: open_ai_compatible
      model: openrouter/free
      baseUrl: https://openrouter.ai/api/v1
      credentialReference: env:OPENROUTER_API_KEY
      timeoutMs: 120000
  roles:
    primary: openrouter
    risk_evaluator: openrouter

sandbox:
  networkDestinations:
    - https://openrouter.ai
```

Validate before the first model turn:

```bash
colossus --config .colossus/config.yaml models route primary
colossus --config .colossus/config.yaml provider doctor
colossus --config .colossus/config.yaml provider models
colossus --config .colossus/config.yaml run "Reply with exactly: connected"
```

See [Configuration](CONFIGURATION.md) for policy, TLS, OPA, sandbox, memory, workflow,
integration, and provider details.

## Development Checks

From the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --locked --manifest-path fuzz/Cargo.toml --all-targets
```

## Frozen Python Users

Python 0.5 remains available only at `python-v0.5.0` and on `python-legacy`.
Fresh Rust installs do not import its configuration, SQLite state, history, or audit
files.

## Next Steps

- Read the [User Guide](USER_GUIDE.md) for the complete command surface.
- Try versioned definitions from [Workflows](WORKFLOWS.md).
- Review [Security](SECURITY.md) before enabling effects.
- Use [Offline and Airgapped Operation](OFFLINE_AIRGAP.md) for isolated deployments.

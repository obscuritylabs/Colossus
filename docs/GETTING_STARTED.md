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
cargo run --offline --manifest-path rust/Cargo.toml \
  -p colossus-cli --bin colossus-rs -- --version
```

## Initialize And Smoke Test

Create strict configuration beside the fresh Rust state:

```bash
colossus --config .colossus/config.yaml config init
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml run "hello"
colossus --config .colossus/config.yaml audit verify
```

The run result is JSON containing `run_id`, `session_id`, `profile: "echo"`,
`output: "hello"`, event count, and elapsed time. `config init` uses the OS
credential store for mandatory journal/signing keys; headless deployments can explicitly
configure environment key references instead. Rust never silently writes plaintext
canonical state.

## Start The REPL

```bash
colossus --config .colossus/config.yaml repl
```

Useful first commands:

```text
/help
/status
/tools
/work
/context status
/exit
```

The Reedline REPL supports durable sessions, encrypted history, streamed assistant/tool
events, multiline input, cursor/draft status, themes, workflows, goals, research,
memories, and authenticated-worker routing.

## Choose A Workspace

The process working directory is the initial workspace. Start Colossus from the
repository you want to operate on, while passing an absolute config path:

```bash
cd ../my-project
colossus --config /absolute/path/to/.colossus/config.yaml repl
```

Restart the REPL from another directory to change the active repository scope.
Filesystem, Git, patch, repository-context, and process effects still require matching
absolute filesystem/executable grants in YAML; changing the process workspace never
expands policy.

## Understand Approvals

One-shot commands default to `deny`; the REPL defaults to `ask`. Approval modes
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

`risk-auto` falls back to an explicit prompt while the risk evaluator is unavailable.
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

From `rust/`:

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

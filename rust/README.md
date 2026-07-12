# Colossus Rust runtime

This directory contains the event-sourced Colossus reconstruction. It intentionally uses
fresh YAML configuration and fresh state; the frozen Python implementation is available
at the `python-v0.5.0` tag and `python-legacy` branch.

The initial alpha implements the contracts, encrypted redb journal, exclusive writer
lease, restartable redb projections and projected repositories, policy gateway, durable
workflow core, and permit-bound filesystem/process/HTTP/provider adapters. macOS and
Linux use a one-shot authenticated Seatbelt/Landlock helper. Windows process execution
uses a per-job AppContainer plus an atomically attached Job Object for network-free
process effects. Network destinations remain fail-closed until an authenticated Windows
proxy transport is accepted; Windows OCI path mapping is still disabled.

The default fresh configuration routes `primary` to the credential-free echo profile,
so the full one-shot agent path is available offline:

```sh
cargo run -p colossus-cli --bin colossus-rs -- run 'Reply with exactly: ok'
cargo run -p colossus-cli --bin colossus-rs -- provider profiles
cargo run -p colossus-cli --bin colossus-rs -- provider doctor
cargo run -p colossus-cli --bin colossus-rs -- provider models
cargo run -p colossus-cli --bin colossus-rs -- models routes
cargo run -p colossus-cli --bin colossus-rs -- models route primary
cargo run -p colossus-cli --bin colossus-rs -- tools list
cargo run -p colossus-cli --bin colossus-rs -- sessions list
cargo run -p colossus-cli --bin colossus-rs -- run --resume 'Continue the latest session'
cargo run -p colossus-cli --bin colossus-rs -- repl
cargo run -p colossus-cli --bin colossus-rs -- preferences show
cargo run -p colossus-cli --bin colossus-rs -- preferences history
cargo run -p colossus-cli --bin colossus-rs -- research run \
  'Summarize the audit architecture' --depth quick --source repo
cargo run -p colossus-cli --bin colossus-rs -- research list
cargo run -p colossus-cli --bin colossus-rs -- telemetry runs
cargo run -p colossus-cli --bin colossus-rs -- telemetry metrics
cargo run -p colossus-cli --bin colossus-rs -- skills list
cargo run -p colossus-cli --bin colossus-rs -- skills show coding
cargo run -p colossus-cli --bin colossus-rs -- run --skill coding 'Implement the scoped change'
cargo run -p colossus-cli --bin colossus-rs -- --approval-mode ask \
  skills scaffold my-skill 'My data-only skill' --resource-dir references
cargo run -p colossus-cli --bin colossus-rs -- skills inspect my-skill
cargo run -p colossus-cli --bin colossus-rs -- skills file-read my-skill SKILL.md
cargo run -p colossus-cli --bin colossus-rs -- skills validate path/to/local-skill --local
cargo run -p colossus-cli --bin colossus-rs -- --approval-mode ask \
  skills install path/to/local-skill
cargo run -p colossus-cli --bin colossus-rs -- --approval-mode ask \
  integrations import-openapi demo openapi.json --base-url https://api.example.test \
  --credential-reference env:DEMO_API_TOKEN
cargo run -p colossus-cli --bin colossus-rs -- integrations list
cargo run -p colossus-cli --bin colossus-rs -- tools list
cargo run -p colossus-cli --bin colossus-rs -- --approval-mode ask \
  integrations connect github --credential-reference env:GITHUB_TOKEN
cargo run -p colossus-cli --bin colossus-rs -- --approval-mode ask \
  integrations connect searxng --base-url http://127.0.0.1:8888 --auth-type none
cargo run -p colossus-cli --bin colossus-rs -- --approval-mode ask \
  integrations connect opensearch --base-url http://127.0.0.1:9200 --auth-type none
cargo run -p colossus-cli --bin colossus-rs -- mcp servers
cargo run -p colossus-cli --bin colossus-rs -- mcp tools --server local
cargo run -p colossus-cli --bin colossus-rs -- --approval-mode ask \
  mcp call local search '{"query":"audit"}'
```

Network profiles use `open_ai_responses` or `open_ai_compatible`, an API-version
`baseUrl`, an optional `env:VARIABLE` credential reference (required for Responses), and
an exact canonical origin in `sandbox.networkDestinations`. Their generation action and
`provider.models` must also be explicitly allowed by the built-in policy. Credentials
are resolved only inside the adapter after authorization. The application loop supports
multiple provider/tool turns, strict schema validation, two bounded malformed-argument
correction turns, and distinct max-turn exhaustion. Incremental transport streaming
remains pending.

To export redacted audit evidence to an existing local directory, configure the
replaceable exporter and grant its write effect explicitly:

```yaml
audit:
  exporter:
    kind: directory
    path: /absolute/path/to/audit-evidence
policy:
  kind: built_in
  allow_actions: [audit.export.write]
sandbox:
  filesystem:
    - root: /absolute/path/to/audit-evidence
      mode: write
```

The directory must already exist. Each deterministic JSON file contains envelope and
chain evidence but no payload ciphertext, nonce, or plaintext. Export work has its own
durable queue position and retry status and is drained automatically by the worker or
manually with:

```sh
cargo run -p colossus-cli --bin colossus-rs -- audit exporter-status
cargo run -p colossus-cli --bin colossus-rs -- audit exporter-drain
cargo run -p colossus-cli --bin colossus-rs -- audit exporter-reset
```

Reset replays deterministic files from the canonical journal and is the explicit
operator recovery for an unknown delivery outcome. The directory adapter is not WORM;
remote/WORM export remains a later adapter.

Tantivy remains the offline memory-index default. To select the disposable Chroma v2
adapter, add a semantic block while retaining canonical memory in the encrypted journal:

```yaml
memory:
  indexEnabled: true
  indexPath: null
  retrievalLimit: 6
  semantic:
    kind: chroma
    baseUrl: http://127.0.0.1:8000
    tenant: default_tenant
    database: default_database
    collection: colossus-memory
    credentialReference: null
    timeoutMs: 30000
    positionPath: .colossus/chroma-position.json
    embedding:
      kind: local
      dimensions: 256
```

Add the Chroma origin to `sandbox.networkDestinations`. Built-in policy must explicitly
allow the outer `memory.*` operation and the matching
`memory.index.chroma.upsert|remove|search|status|reset` action. Rebuild reset can instead
be approval-gated. For a remote embedding profile, replace the nested block with
`kind: open_ai_compatible`, `profile`, `model`, `baseUrl`, optional
`credentialReference`, `timeoutMs`, and optional `dimensions`; add its origin and allow
`embedding.openai.create`. Chroma and embedding requests are separately audited and fail
closed. `memory index status|sync|rebuild` exposes readiness, journal position, lag, and
bounded adapter errors; canonical search falls back when the projection is unavailable.
An unknown Chroma mutation outcome is persisted locally and blocks automatic retries;
run an operator-authorized `memory index rebuild` to reset and reconstruct the disposable
projection from canonical journal state.

Fresh config enables only the pure `echo` tool. Configure `agent.maxTurns` in `1..=100`
and select exact active names with `tools list`. File, process, HTTP, memory, work, and
subagent tools remain subject to their policy actions and resource obligations.

Durable research uses `research.maxSources` (1..=100) and `research.maxWorkers`
(1..=16). Add `research.run` to the built-in policy before invoking it. Repository
collection is read-only and each search independently crosses the gateway and
post-effect release check. Configure a SearXNG JSON endpoint with
`research.search.kind: searxng`, add its exact origin to
`sandbox.networkDestinations`, and allow `network.http` to enable the web lane.
Unconfigured web and MCP lanes are retained as explicit limitations. Configured MCP
research tools use the same allowlist, schema validation, approval, sandbox, quarantine,
and post-effect release path as terminal and model calls.
`research_planner`, `research_worker`, and `research_synthesizer` use normal
gateway-bound provider roles; invalid or unavailable model output produces a durable
deterministic fallback instead of weakening citation checks. Source labels, claims,
lane/progress outcomes, and the final cited report are reconstructed from the encrypted
journal. The report is also appended to its session, and abandoned runs become
`interrupted` on restart without implicit retry.

`telemetry runs`, `telemetry show RUN_OR_UNIQUE_PREFIX`, and `telemetry metrics` derive
duration and operational counters from persisted typed events. Timelines expose only
envelope metadata, lineage ids, hashes, and encrypted sizes; raw prompts, model text,
tool output, and decrypted research evidence are never returned.

Declarative skills are discovered from ordered bundled, repository, and user libraries.
Later roots cannot override earlier names unless `skills.allowUserOverrides` is enabled.
`@skill:name`, `--skill NAME`, and REPL `/skill use NAME` activate bounded instructions;
required tools must already exist in the configured catalog and activation never grants
policy permission. `skill.resource.list/read` work only for skills active on the current
run, cross the effect gateway and post-effect release gate, reject traversal and symlinks,
and return scripts only as bounded UTF-8 text. Authoring mutates only the configured user
library: scaffold, write, and install require approval by default; existing files require
the SHA-256 returned by `skills file-read`; candidates are validated before replacement;
and local install sources must be traversal-free, symlink-free workspace directories.
Executable activation belongs exclusively to verified packs.

Capability packs are managed with
`packs list|show|verify|validate|install|enable|disable|uninstall` and publisher/key trust
with `packs trust list|add`. Pack verification rejects traversal, symlinks, undeclared
payloads, hash/size mismatches, excessive permissions, and invalid Ed25519 signatures.
Installed lifecycle and publisher trust are reconstructed from the encrypted journal.
Enabled pack skills are reverified and added to skill discovery on the next runtime
start. Local OCI-layout tar and tar+gzip pack sources are descriptor-verified and
bounded before extraction. Enabled fixed-argument tools and MCP servers are reverified,
added to their normal runtime registries on restart, permission-restricted per declared
capability, and executed only through approval, one-use permits, sandboxing, quarantine,
post-effect policy, audit, and credential redaction.

Signed offline releases use `bundle build`, `bundle verify`, and `bundle install`.
Build deterministically copies a staged multi-target payload, late-resolves an
environment signing-seed reference, requires matching publisher/key trust, signs and
re-verifies the completed bundle, then publishes atomically. Install fully re-verifies
offline, selects only the current native target, and creates a clean-prefix executable
without clobbering an existing path. Both mutations require approval plus matching
filesystem obligations.

OpenAPI 3 JSON imports are compiled into strict `openapi.NAME.OPERATION` tools and
persisted as immutable integration lifecycle events. Path, query, and JSON-body fields
become operation arguments; authentication never does. Missing environment credentials
produce `pending_auth` connections whose tools remain hidden. Connected calls require
approval by default, enforce the configured exact origin and output/timeout bounds,
resolve credential handles only inside the permit-bearing adapter, quarantine every
response for post-effect policy, and redact an exact credential value if an upstream
echoes it. Re-import to change a connection; disconnecting appends history rather than
deleting it.

Native GitHub, SearXNG, and OpenSearch connections use the same lifecycle and gateway.
GitHub exposes repository, issue, pull-request, check, and release reads. SearXNG exposes
normalized search and health results. OpenSearch exposes cluster/index discovery,
document search/retrieval, and independently authorized index/update/delete mutations.
Bearer/API-key secrets and OpenSearch Basic username/password values are resolved only
after authorization; canonical state and policy inputs retain their `env:` handles.

Configured MCP uses exact stdio server executables and the official Rust SDK protocol
models. Every server declares literal argv, an optional working directory, child
environment names mapped to `env:HOST_VARIABLE` references, exact allowed tools, and
optional research call templates. The command must also appear in
`sandbox.executables`; the working directory needs a filesystem grant; child environment
names must appear in `sandbox.environment`. Discovery is permit-bound and allowed by the
built-in policy without a prompt, while invocation requires approval by default. Tools
remain absent from the model catalog until at least one server is configured.

```yaml
mcp:
  servers:
    local:
      command: /absolute/path/to/mcp-server
      args: []
      workingDirectory: /absolute/path/to/workspace
      environment:
        API_TOKEN: env:LOCAL_MCP_TOKEN
      allowedTools: [search]
      researchTools:
        - tool: search
          title: Local MCP search
          arguments: {query: "{query}"}
      timeoutMs: 30000
      maxOutputBytes: 1048576
```

Normal runs create a durable session automatically and return its id. Use
`run --session ID` for an exact session, `run --resume` for the most recently updated
session, and `sessions list|show|messages|new` for discovery. The REPL keeps an active session and
offers `/resume` as a numbered picker while retaining `/session resume ID` as the exact
escape hatch. Message bodies stay in the encrypted journal; projections contain bounded
session summaries only.

The REPL persists strict presentation preferences and submitted history entries in the
encrypted event journal, hydrating only the newest 1,000 into Reedline. Each change
crosses the effect gateway and uses authenticated worker IPC when a worker owns the state
lease; Rust never writes a plaintext Reedline history sidecar. Use
`/theme NAME`, `/theme preview NAME`, `/theme save NAME`, `/theme reset`,
`/stream on|raw|off`,
`/events compact|verbose|off`, `/reasoning on|off`,
`/transcript comfortable|compact`, `/multiline on|off|toggle`, `/trace`, and
`/repl prefs|save|reset`. These settings affect terminal rendering only. Work and context
status use semantic summaries, safe provider reasoning summaries remain independently
toggleable, and `raw` means normalized visible text without semantic event blocks—not
unredacted provider frames or hidden reasoning.

Streamed runs use correlated `RunEventEnvelope` values in both embedded mode and
authenticated worker protocol v2. Run start/completion and tool start/completion are
durable before display. Compact rendering uses distinct file, shell, Git, work, context,
repository, skill, web, MCP, trace, integration, pack, and generic summaries; verbose
mode adds bounded arguments/results and run metadata. Terminal errors always state
whether recovery continues, and activity lines include the current phase/action and
elapsed time. Interactive terminals refresh active elapsed time in place; redirected
streams remain stable and escape-free. The embedded and worker REPLs also share a cached
status prompt showing session, resolved primary model/profile, context/messages, work,
approval mode, display preferences, and last run status.
The prompt also derives 1-based cursor line/column and Unicode-aware draft
character/line counts locally from Reedline's repaint pass. Draft text is not sent over
worker IPC or persisted per keystroke.

The five built-in data-only palettes style Reedline prompt segments, assistant text,
semantic event labels, and animated activity frames only on an interactive terminal.
Custom `.json` and `.toml` files load from the directory beside the selected config
(`themes/`) and the platform config directory (`colossus/themes/`). Files are limited to
64 KiB and libraries to 64 custom themes; symlinks, unknown fields, invalid colors,
duplicates, and built-in name collisions fail closed. Versioned files use this shape:

```json
{
  "schemaVersion": 1,
  "name": "ocean",
  "base": "default",
  "title": "Ocean",
  "caret": ">",
  "continuation": "|",
  "prompt": {"left": "#00ffff", "indicator": "#00d7ff"},
  "styles": {
    "assistant": {"foreground": "#d7ffff"},
    "tool": {"foreground": "#00afff", "bold": true}
  },
  "spinner": "line"
}
```

Allowed spinners are `dots`, `line`, `arc`, `bouncingBar`, and `aesthetic`. Selecting a
custom theme persists its fully resolved palette and SHA-256 source hash, so restarts do
not depend on the file remaining unchanged. Rust also strictly maps the frozen Python
data-only theme schema during cutover.
Redirected and authenticated scripted output remains ANSI-free. The legacy Rust alpha
value `plain` is accepted as an alias for `mono` when loading state or handling commands.

Inspect sandbox readiness or run an explicitly configured exact executable:

```sh
cargo run -p colossus-cli --bin colossus-rs -- sandbox doctor
cargo run -p colossus-cli --bin colossus-rs -- process run /bin/echo --cwd . -- hello
```

Process and network actions are deny-by-default. Add the exact action, executable,
filesystem grants, environment names, and canonical HTTP(S) origins to the fresh YAML
configuration before invoking them.

The external-runtime acceptance suites are ignored during ordinary offline tests. Run
them explicitly with a preloaded immutable image and local OPA/OpenSSL binaries:

```sh
COLOSSUS_OCI_RUNTIME=/absolute/path/to/docker-or-podman \
COLOSSUS_OCI_IMAGE='python:3.13-slim@sha256:...' \
COLOSSUS_OCI_PROXY_IMAGE='sha256:...' \
cargo test -p colossus-cli --test oci_sandbox -- --ignored

COLOSSUS_OPA_BIN=/absolute/path/to/opa \
COLOSSUS_OPENSSL_BIN=/absolute/path/to/openssl \
cargo test -p colossus-policy --test opa_live -- --ignored

COLOSSUS_CHROMA_URL=http://127.0.0.1:8000 \
cargo test -p colossus-memory-chroma --test chroma_live -- --ignored
```

`COLOSSUS_OCI_PROXY_IMAGE` is the immutable ID of the preloaded scratch image built from
`oci-proxy.Dockerfile`; CI builds its static musl binary and loads the same image into
Docker and Podman. OCI executable grants are exact normalized paths inside the immutable
workload image, while bind-mounted filesystem grants remain canonical host paths.
For a networked OCI configuration, set `sandbox.ociProxyImage` to that immutable image
ID or repository digest, list canonical origins under `sandbox.networkDestinations`, and
use `sandbox.timeoutMs: 10000` or higher. Network-free OCI configurations may leave
`ociProxyImage` null and retain the five-second minimum.

CI runs the network-off and proxy-only OCI suite against both Docker and Podman, runs
live OPA/mTLS separately, checks the Chroma v2 lifecycle against pinned current and
previous releases, and compiles all targets on macOS and Windows.

### Release artifacts

The `rust-release-smoke` CI matrix produces these native artifacts:

| Platform | Targets | Archive |
| --- | --- | --- |
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | `.tar.gz` |
| Linux (static musl) | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` | `.tar.gz` |
| Windows | `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc` | `.zip` |

Every target is built and executed on a matching native runner. Before upload, CI runs
`--version`, strict config parsing, a credential-free echo turn, and `audit verify` using
[`release/smoke-config.yaml`](release/smoke-config.yaml). It then packages the executable
as `colossus`/`colossus.exe` with the platform installer, license, and this README,
then writes a SHA-256 sidecar. After creating the archive, CI extracts it into an empty
directory, installs into an empty prefix, and repeats the offline echo/audit smoke using
only the installed executable. The Linux jobs also reject artifacts that `file` does not
identify as statically linked.

After verifying the archive checksum, install without Cargo or Python:

```bash
tar -xzf colossus-VERSION-TARGET.tar.gz
./colossus-VERSION-TARGET/install.sh
```

```powershell
Expand-Archive colossus-VERSION-TARGET.zip
.\colossus-VERSION-TARGET\install.ps1
```

The Unix default prefix is `$HOME/.local`; Windows uses the same logical default.
Add the prefix's `bin` directory to `PATH`. Installers reject linked package binaries and
linked destination `bin` directories, copy through a destination-local temporary name,
and make no network requests. Checksums detect transfer corruption; use signed bundle
verification when authenticity is required.

The separate runtime matrices execute native Seatbelt/Landlock acceptance on macOS and
Linux arm64/x64 and the authenticated worker suite over Windows named pipes on Windows
arm64/x64. The Unix suite treats unavailable kernel isolation as a failure and exercises
filesystem traversal, environment, descendant, process-count, memory, timeout, proxy,
and raw-egress boundaries. Windows `windows_job` execution uses a per-effect AppContainer
identity for policy-root filesystem and default-deny network isolation plus an atomically
attached Job Object for descendant, timeout, process-count, and aggregate-memory limits.
Native x64 and arm64 CI exercises those boundaries. Non-empty Windows network grants fail
closed until an authenticated AppContainer proxy transport exists.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The stable workspace suite includes the committed fuzz corpus through
`colossus-fuzzing`. Full mutation fuzzing uses the independent `fuzz/` workspace so the
nightly sanitizer toolchain and `libfuzzer-sys` never enter the production lockfile.
Install nightly plus `cargo-fuzz` 0.13.2, then run:

```sh
cargo fuzz run contracts_json
cargo fuzz run workflow_yaml
cargo fuzz run workflow_condition
```

CI pins its nightly toolchain, executes 5,000 inputs per target, and uploads any crash
artifacts. Add minimized reproductions to the matching `fuzz/corpus/` directory before
fixing a discovered defect.

Supply-chain CI pins `cargo-deny` 0.20.2 and `cargo-audit` 0.22.2. It checks the
production and independent fuzz lockfiles separately; duplicate transitive versions are
reported, while unapproved licenses, wildcard requirements, unknown sources, banned
crates, yanked or unmaintained dependencies, and RustSec advisories fail the build.

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo install cargo-audit --version 0.22.2 --locked

cargo deny --locked check -A license-not-encountered licenses sources bans
cargo deny --locked check -D warnings advisories
cargo audit -D warnings --file Cargo.lock

cargo deny --manifest-path fuzz/Cargo.toml --config deny.toml --locked check -A license-not-encountered licenses sources bans
cargo deny --manifest-path fuzz/Cargo.toml --config deny.toml --locked check -D warnings advisories
cargo audit -D warnings --file fuzz/Cargo.lock
```

# Colossus Rust runtime

This directory contains the event-sourced Colossus reconstruction. It intentionally uses
fresh YAML configuration and fresh state; the frozen Python implementation is available
at the `python-v0.5.0` tag and `python-legacy` branch.

The initial alpha implements the contracts, encrypted redb journal, exclusive writer
lease, restartable redb projections and projected repositories, policy gateway, durable
workflow core, and permit-bound filesystem/process/HTTP/provider adapters. macOS and
Linux use a one-shot authenticated Seatbelt/Landlock helper. Windows process execution
remains fail-closed until OCI path mapping and the live Windows acceptance suite are
complete.

The default fresh configuration routes `primary` to the credential-free echo profile,
so the full one-shot agent path is available offline:

```sh
cargo run -p colossus-cli --bin colossus-rs -- run 'Reply with exactly: ok'
cargo run -p colossus-cli --bin colossus-rs -- provider profiles
cargo run -p colossus-cli --bin colossus-rs -- provider doctor
cargo run -p colossus-cli --bin colossus-rs -- provider models
cargo run -p colossus-cli --bin colossus-rs -- models routes
cargo run -p colossus-cli --bin colossus-rs -- tools list
cargo run -p colossus-cli --bin colossus-rs -- sessions list
cargo run -p colossus-cli --bin colossus-rs -- run --resume 'Continue the latest session'
cargo run -p colossus-cli --bin colossus-rs -- research run \
  'Summarize the audit architecture' --depth quick --source repo
cargo run -p colossus-cli --bin colossus-rs -- research list
```

Network profiles use `open_ai_responses` or `open_ai_compatible`, an API-version
`baseUrl`, an optional `env:VARIABLE` credential reference (required for Responses), and
an exact canonical origin in `sandbox.networkDestinations`. Their generation action and
`provider.models` must also be explicitly allowed by the built-in policy. Credentials
are resolved only inside the adapter after authorization. The application loop supports
multiple provider/tool turns, strict schema validation, two bounded malformed-argument
correction turns, and distinct max-turn exhaustion. Incremental transport streaming
remains pending.

Fresh config enables only the pure `echo` tool. Configure `agent.maxTurns` in `1..=100`
and select exact active names with `tools list`. File, process, HTTP, memory, work, and
subagent tools remain subject to their policy actions and resource obligations.

Durable research uses `research.maxSources` (1..=100) and `research.maxWorkers`
(1..=16). Add `research.run` to the built-in policy before invoking it. Repository
collection is read-only and each search independently crosses the gateway and
post-effect release check. Configure a SearXNG JSON endpoint with
`research.search.kind: searxng`, add its exact origin to
`sandbox.networkDestinations`, and allow `network.http` to enable the web lane.
Unconfigured web and MCP lanes are retained as explicit limitations.
`research_planner`, `research_worker`, and `research_synthesizer` use normal
gateway-bound provider roles; invalid or unavailable model output produces a durable
deterministic fallback instead of weakening citation checks. Source labels, claims,
lane/progress outcomes, and the final cited report are reconstructed from the encrypted
journal. The report is also appended to its session, and abandoned runs become
`interrupted` on restart without implicit retry.

Normal runs create a durable session automatically and return its id. Use
`run --session ID` for an exact session, `run --resume` for the most recently updated
session, and `sessions list|show|messages|new` for discovery. The REPL keeps an active session and
offers `/resume` as a numbered picker while retaining `/session resume ID` as the exact
escape hatch. Message bodies stay in the encrypted journal; projections contain bounded
session summaries only.

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
live OPA/mTLS separately, and compiles all targets on macOS and Windows.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

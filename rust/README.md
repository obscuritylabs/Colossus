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

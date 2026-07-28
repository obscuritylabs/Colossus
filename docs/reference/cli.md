---
title: CLI reference
description: Global options, every public command route, defaults, and machine-output contracts.
audience: developer
type: reference
---

# CLI reference

```text
colossus [OPTIONS] <COMMAND>
```

## Global options

| Option | Values | Default | Meaning |
| --- | --- | --- | --- |
| `-w`, `--workspace PATH` | Existing directory | Current directory | Select and canonicalize the repository workspace |
| `--config PATH` | YAML path | `.colossus/config.yaml` | Select configuration |
| `--approval-mode MODE` | `deny`, `ask`, `risk-auto`, `full-access` | See below | Satisfy existing approval obligations |
| `--output FORMAT` | `auto`, `human`, `json` | `auto` | Select structured output rendering |
| `--no-alt-screen` | Flag | Off | Run the TUI in an inline viewport |
| `-h`, `--help` | Flag | — | Show command help |
| `-V`, `--version` | Flag | — | Show binary version |

Global options appear before the command:

```bash
colossus -w /absolute/path/to/repository \
  --config .colossus/config.yaml \
  --approval-mode ask workflow list
```

Relative `--config` paths resolve against `--workspace`. Embedded runtime callers retain
current-directory behavior unless they opt into the explicit workspace-aware open API.
An active worker publishes its canonical workspace and rejects a client selecting a
different one.

Interactive TUI and the long-running bare `worker` default to `ask`. Other commands
executed in-process default to `deny`; when an active worker handles a command, that
worker's configured approval mode applies. Approval mode satisfies an existing approval
obligation—it never changes an access or policy decision.

`risk-auto` is eligible only for model and child-agent `shell.run`, `web.search`, and
bodyless `network.http` GET effects outside workflows. A low-risk `allow`
recommendation produces a request-bound proof; other network methods and every
non-low-risk assessment fall back to explicit approval or denial.
Each automatic grant emits a human-readable **Automatic approval review** notice on the
attached terminal or TUI without opening an approval prompt.
An unavailable evaluator or invalid assessment emits an **Automatic approval review
failed** warning before Colossus falls back to explicit approval. The warning identifies
the sanitized failure category without printing raw provider diagnostics or malformed
model output.

## Command groups

| Command | Purpose |
| --- | --- |
| `config` | Create and inspect strict YAML configuration |
| `audit` | Verify, inspect, anchor, and export journal evidence |
| `policy` | Diagnose built-in or OPA policy |
| `projection` | Inspect, drain, or rebuild disposable projections |
| `state` | Diagnose canonical storage, lease, repositories, and projection readiness |
| `sandbox` | Diagnose native, OCI, or platform isolation |
| `process` | Execute one exact program without an implicit shell |
| `network` | Perform policy-allowed brokered HTTP requests |
| `workflow` | Validate, register, run, trigger, recover, and inspect workflows |
| `provider` | Inspect and diagnose model profiles |
| `search` | Inspect and query provider-neutral search routes |
| `models` | Inspect role-to-profile routing |
| `artifacts` | Upload, inspect, and download caller-owned released artifacts |
| `tools` | Inspect the resolved strict tool catalog |
| `sessions` | Create and inspect durable sessions; `run` and TUI attach or resume |
| `work` | Render bounded actionable work for a session |
| `preferences` | Inspect or reset presentation preferences |
| `context` | Inspect, compact, snapshot, and restore long-session context |
| `tasks` | Create and inspect session tasks |
| `decisions` | Create and inspect binding key decisions |
| `plans` | Create, inspect, and approve plans; `run --execute-plan` executes |
| `goals` | Run and inspect bounded goals |
| `agents` | Inspect and control durable child-agent jobs |
| `memories` | Create, search, archive, supersede, and index memories |
| `research` | Run and inspect source-backed research |
| `telemetry` | Inspect metadata-only run telemetry |
| `skills` | Discover, compose, author, validate, and install data-only skills |
| `packs` | Verify and lifecycle-manage signed capability packs |
| `collections` | Build, verify, and install signed collections |
| `registry` | Pull and push authenticated collection transports |
| `bundle` | Build, verify, and install signed offline bundles |
| `integrations` | Manage persisted integrations and imported OpenAPI tools |
| `mcp` | Discover and invoke configured MCP servers |
| `run` | Execute one audited model turn through a configured role |
| `echo` | Run the credential-free, network-free smoke provider |
| `tui` | Start the interactive terminal interface |
| `worker` | Own the writer lease, host the public API, and administer application credentials offline |

## Complete route index

Every public leaf route in the current binary appears below. Arguments in uppercase are
positional:

| Group | Leaf routes |
| --- | --- |
| `config` | `init [--access-profile PROFILE] [--sandbox-profile PROFILE]`, `show`, `effective` |
| `audit` | `verify`, `show`, `export`, `anchor-status`, `exporter-status`, `exporter-drain`, `exporter-reset` |
| `policy` | `doctor` |
| `projection` | `status`, `drain`, `rebuild [NAME]` |
| `state` | `doctor` |
| `sandbox` | `doctor` |
| `process` | `run EXECUTABLE [-- ARGS...]` |
| `network` | `get URL` |
| `workflow` | `validate PATH`, `register PATH`, `list`, `show NAME VERSION`, `run NAME VERSION`, `status RUN_ID`, `resume RUN_ID`, `input RUN_ID INPUT`, `cancel RUN_ID` |
| `workflow schedule` | `create SCHEDULE_ID NAME VERSION`, `list`, `show SCHEDULE_ID`, `enable SCHEDULE_ID`, `disable SCHEDULE_ID`, `tick` |
| `workflow webhook` | `create WEBHOOK_ID NAME VERSION`, `list`, `show WEBHOOK_ID`, `enable WEBHOOK_ID`, `disable WEBHOOK_ID`, `ingest WEBHOOK_ID`, `serve` |
| `workflow subscription` | `create SUBSCRIPTION_ID NAME VERSION`, `list`, `show SUBSCRIPTION_ID`, `enable SUBSCRIPTION_ID`, `disable SUBSCRIPTION_ID`, `tick` |
| `provider` | `profiles`, `doctor [PROFILE] [--include-provider-response]`, `models [PROFILE]` |
| `search` | `profiles`, `query QUERY` |
| `models` | `profiles`, `doctor [PROFILE] [--include-provider-response]`, `routes`, `route [ROLE]` |
| `artifacts` | `upload PATH`, `show ARTIFACT_ID`, `download ARTIFACT_ID OUTPUT` |
| `tools` | `list` |
| `sessions` | `list`, `show SESSION_ID`, `messages SESSION_ID`, `new [TITLE]` |
| `work` | `work [--session SESSION_ID]` |
| `preferences` | `show`, `history`, `reset` |
| `context` | `status SESSION_ID`, `list SESSION_ID`, `compact SESSION_ID`, `restore SESSION_ID SNAPSHOT_ID` |
| `tasks` | `list [--session SESSION_ID]`, `show TASK_ID`, `create SESSION_ID TITLE`, `update TASK_ID` |
| `decisions` | `list [--session SESSION_ID]`, `show DECISION_ID`, `create SESSION_ID TITLE DECISION`, `update DECISION_ID`, `archive DECISION_ID`, `supersede DECISION_ID TITLE DECISION` |
| `plans` | `list`, `show PLAN_ID`, `create SESSION_ID PROMPT`, `approve PLAN_ID` |
| `goals` | `list`, `show GOAL_ID`, `run OBJECTIVE --session SESSION_ID` |
| `agents` | `queue SESSION_ID TASK`, `list`, `show JOB_ID`, `status`, `drain`, `cancel JOB_ID`, `requeue JOB_ID` |
| `memories` | `list`, `show MEMORY_ID`, `search QUERY`, `create TEXT`, `archive MEMORY_ID`, `supersede MEMORY_ID TEXT` |
| `memories index` | `status`, `sync`, `rebuild` |
| `research` | `run QUESTION`, `list`, `show RUN_ID`, `sources RUN_ID`, `claims RUN_ID` |
| `telemetry` | `runs`, `show RUN_ID`, `metrics` |
| `skills` | `list`, `show NAME`, `duplicates`, `compose PROMPT`, `scaffold NAME DESCRIPTION`, `inspect NAME`, `file-read NAME PATH`, `write NAME PATH CONTENT`, `validate TARGET`, `install PATH`, `resources NAME`, `read NAME PATH` |
| `packs` | `list`, `show NAME`, `verify PATH`, `validate PATH`, `install PATH`, `enable NAME`, `disable NAME`, `uninstall NAME`, `call TOOL` |
| `packs trust` | `list`, `add PUBLISHER --public-key KEY` |
| `collections` | `verify PATH`, `build SOURCE DESTINATION`, `install PATH` |
| `registry` | `pull URL DESTINATION`, `push PATH URL` |
| `bundle` | `key-info`, `verify PATH`, `build SOURCE DESTINATION`, `install PATH --prefix PATH` |
| `integrations` | `list`, `show NAME`, `connect NAME`, `import-openapi NAME SPEC`, `disconnect NAME`, `call TOOL ARGUMENTS` |
| `mcp` | `servers`, `tools`, `call SERVER TOOL ARGUMENTS` |
| Top-level execution | `run [PROMPT]`, `echo MESSAGE`, `tui`, `worker` |

## Important defaults and bounds

| Route | Default or constraint |
| --- | --- |
| `audit show` | `--from 1`, `--limit 100` |
| `audit export` | `--from 1`, `--limit 1000` |
| `config init` | `--access-profile development`; sandbox defaults to `workspace-development` for development and `offline-default` otherwise |
| `process run` | Exact executable; `--cwd .`; `--env KEY=VALUE` repeats; arguments after `--` are literal |
| `workflow run` | `--inputs {}`; foreground unless `--queued` |
| `workflow schedule create` | cadence required in `60..=2678400`; `--misfire fire-once`; enabled by default |
| Schedule, webhook, subscription `list` | `--limit 100` |
| `workflow webhook create` | `--replay-window-seconds 300`; `--max-body-bytes 1048576`; enabled by default |
| `workflow webhook serve` | `--bind 127.0.0.1:8787` |
| `search query` | `--role agent`; `--limit 10` |
| `sessions list`, `research list`, `telemetry runs` | `--limit 20` |
| Most record lists | `--limit 100` |
| `goals run` | `--role primary`; `--max-iterations 5` in `1..=50` |
| `agents queue` | `--role subagent_default` |
| `research run` | `--depth standard`; planned-query budgets are `quick=1`, `standard=3`, `deep=6`; `--source repo,web,mcp` |
| `artifacts upload` | Policy-authorized bounded files; `--purpose run-input`; encrypted bytes are owner-bound to the CLI application identity |
| `run` | `--role primary`; `--goal-max-iterations 5`; fresh session unless `--session` or `--resume`; `--attach PATH` repeats up to 16 policy-read files within a 1 MiB aggregate UTF-8 input bound |
| `tui` | fresh session unless `--session` or `--resume` |
| `worker` | serves authenticated local IPC; add `--public-api-dir ABS_OWNER_PRIVATE_DIR` to host authenticated loopback gRPC; `--once`, `--status`, `--shutdown`, enrollment, and revocation modes conflict |

## Worker public API flags

The installed worker can publish `colossus.api.v1alpha1` on an ephemeral
`127.0.0.1` port:

```bash
colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api"
```

The directory must be absolute. The Unix-native implementation creates only the final
directory component when needed and requires the canonical directory to be owned by
the current user with mode `0700`. It publishes fixed owner-only files
`endpoint.json` and `certificate.pem`; `.public-api.lock` prevents a second CLI worker
from claiming the same discovery directory. These files contain public discovery and
certificate material only.

Application enrollment is an explicit offline worker action:

```bash
colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api" \
  --enroll-application app:example-desktop \
  --scope runs:execute \
  --scope runs:read \
  --scope runs:control \
  --scope prompts:respond \
  --role primary \
  --credential-keyring-service com.example.desktop \
  --credential-keyring-account colossus-public-api
```

`--scope` and `--role` must each appear at least once. Scope values are exact and must
be one of `runs:execute`, `runs:read`, `runs:control`, `prompts:respond`, or
`approvals:respond`. Repeat `--tool EXACT_TOOL_NAME` to grant tool ceilings; omitting
it denies every tool. `agent.delegate` is rejected until public application authority
can be propagated safely to child runs. Enrollment refuses to run while a private
worker endpoint or journal writer exists.

The bearer is written directly to the named OS-keyring entry. It is never printed and
there is no flag for bearer input. The non-secret enrollment result includes the
stable instance ID and certificate SHA-256; provision those values independently into
the application and never trust a pin obtained only from the discovery directory. An
existing destination is rejected unless
`--replace-credential` is supplied. Before any mutation, replacement authenticates the
stored credential under this public API root and requires the same application ID.
Malformed, revoked, foreign-root, or other-application entries fail closed.
Replacement then issues the new credential as pending, stores it, durably activates
it, and durably revokes the prior credential in that order. Pending credentials cannot
authenticate. The result includes both non-secret credential IDs. Use explicit
revocation for credentials stored under other keyring entries:

```bash
colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api" \
  --revoke-credential 018f0000-0000-7000-8000-000000000001
```

Revocation is durable and idempotent. If a newly issued bearer cannot be written or
durably activated, the CLI immediately revokes it and restores the prior destination
when applicable before returning an error. If prior-credential revocation cannot be
confirmed after replacement activation, the CLI preserves the active new credential
at the destination rather than risk restoring an already-revoked token. The sanitized
error includes both non-secret credential IDs and instructs the administrator to
reconcile and explicitly revoke the prior credential, never either bearer.
Native public API directory enforcement currently fails closed on non-Unix platforms.

`run --role` accepts any configured model role, not only `primary`. `--session` and
`--resume` conflict. Plan creation and execution also conflict:
`run --plan` creates a draft, while `run --execute-plan PLAN_ID` consumes an approved
plan; add `--goal` to execute that plan through bounded Goal Mode.

Clap help is the executable authority for every flag and value:

```bash
colossus COMMAND --help
colossus COMMAND SUBCOMMAND --help
```

## JSON output contracts

With `--output json`, one JSON value is written to stdout; diagnostics and optional
streamed text stay on stderr. Lists are arrays unless the command returns a named page
or status contract. Important roots are:

| Commands | Root fields |
| --- | --- |
| `run` | `run_id`, nullable `session_id`, `role`, `profile`, `model`, `output`, `event_count`, `elapsed_seconds` |
| `provider doctor` | `profile`, `provider`, `ready`, `tool_calls`, `streaming`, `checks` |
| `search query` | `query`, `count`, `results`; each result has `rank`, `title`, `url`, `snippet`, nullable `source` |
| `workflow status` | `run_id`, workflow identity/hash, parent/trigger linkage, `call_depth`, `status`, `inputs`, nullable `outputs`, completion/wait fields |
| `sessions show` | `id`, nullable `title`, timestamps, `message_count`, nullable `last_run_id`, nullable `last_user_preview` |
| `research show` | `id`, `session_id`, question/depth/source lanes, `status`, queries, progress, limitations, report/error, timestamps |
| `telemetry metrics` | Aggregated run/tool/provider/context counters and duration totals |
| `tools list` | Active schemas plus source/risk metadata, canonical workspace, sandbox profile, action decision, and bounds |
| `config effective` | Active/hidden tools and actions plus explicit/derived grants, resolved shell, protected paths, wildcard meaning, and unmet prerequisites |
| `worker --enroll-application` | Non-secret application ID, new credential ID, stable instance ID and certificate SHA-256 trust anchor, exact scopes/role/tool ceilings, destination keyring identifiers, replacement flag, and nullable revoked prior credential ID |
| `worker --revoke-credential` | Non-secret credential ID and durable revocation result |

Optional values are JSON `null`; enums and tagged states use documented lowercase
snake-case strings. `config show` is the deliberate exception: it emits strict YAML so
references can be reviewed intact. `audit export` emits one redacted JSON object per
line. Do not parse human or TUI rendering.

Provider and model Doctor commands remain status-only by default. With
`--include-provider-response`, a non-success check may add `provider_response` containing
the exact credential-free request URL and JSON body plus at most 16 KiB of response body,
the response status and content type, encoding information, and a truncation marker. The
configured provider credential is replaced with `[REDACTED]`. This explicit diagnostic
output crosses post-effect policy but is not attached to ordinary runs, TUI events, or
durable audit payloads.

## Common routes

```bash
colossus -w /path/to/repository config init
colossus -w /path/to/repository config effective
colossus -w /path/to/repository run "Summarize this repository"
colossus -w /path/to/repository --approval-mode risk-auto tui
colossus sessions list
colossus workflow list
colossus audit verify
```

Machine consumers should pass `--output json` and treat the command's documented JSON
shape as the contract. Human output is optimized for reading and may change presentation
without changing application semantics.

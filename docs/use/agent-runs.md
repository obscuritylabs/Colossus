---
title: Agent runs
description: Run bounded Colossus agent turns interactively or as stable machine-readable output.
audience: user
type: how-to
---

# Agent runs

## Goal

Choose the right one-shot run mode, control its bounds, and capture either a human result
or stable JSON without mixing streamed events into stdout.

## Prerequisites

- An initialized configuration.
- A working provider route. The offline `echo` route is sufficient for the examples.
- Any tools required by the prompt visible in `config effective`, with matching policy
  and sandbox grants.

## Steps

### 1. Run one prompt

```bash
colossus -w /absolute/path/to/repository \
  --config .colossus/config.yaml run \
  "Summarize this repository"
```

Interactive stdout contains only the Markdown-capable assistant response. Piped or
redirected stdout defaults to the complete stable JSON result.

### 2. Set explicit bounds when needed

```bash
colossus --config .colossus/config.yaml run --max-turns 12 \
  "Inspect the problem, implement the smallest change, and verify it"
```

Use `--role ROLE` to select an operator-configured model role. The role chooses a route;
the model cannot choose an endpoint or credential.

The sparse default is immediately usable `allow_all` plus acknowledged full host
access. For a narrower development session, explicitly select `access.profile:
development` and `sandbox.profile: workspace-development`, then satisfy each execution
approval interactively or with a reviewed mode:

```bash
colossus -w /absolute/path/to/repository \
  --config .colossus/config.yaml \
  --approval-mode risk-auto run \
  "Inspect the failing tests, implement the smallest fix, and verify it"
```

`risk-auto` can produce a request-bound proof for a low-risk `shell.run`, `web.search`,
bodyless `network.http` GET, or configured top-level `mcp.call` outside workflow
lineage. MCP review receives credential-free metadata for the exact freshly discovered
call; descriptions and annotations remain untrusted hints. It does not apply to
workspace mutations, non-read-only network methods, integrations, pack-provided MCP
actions, workflows, or system actors.
When it grants a proof, Colossus emits an **Automatic approval review** notice with the
reviewed action, resource, low-risk result, authorization mode, and reason.
If the evaluator is unavailable or returns an invalid assessment, Colossus emits an
**Automatic approval review failed** warning and then requests explicit approval. The
warning is sanitized and does not echo raw provider output.

### 3. Stream released progress

```bash
colossus --config .colossus/config.yaml run --stream \
  "Inspect the active tool surface"
```

Released deltas and events go to stderr. The final selected result remains on stdout, so
redirecting stdout still produces a clean artifact:

```bash
colossus --config .colossus/config.yaml --output json \
  run --stream "Report repository status" > result.json
```

For a private CLI run, attach bounded UTF-8 workspace files directly:

```bash
colossus run --attach design.md --attach src/lib.rs \
  "Review the attached files and identify inconsistent assumptions"
```

Attachment paths are sent to the active runtime, which performs each read through the
normal filesystem policy and audit boundary. The CLI never pre-reads attachment content
to bypass workspace restrictions.

For reusable opaque content, upload through the encrypted artifact service:

```bash
colossus artifacts upload design.md
colossus artifacts show ARTIFACT_ID
colossus artifacts download ARTIFACT_ID restored-design.md
```

Artifact commands preserve only the display name and declared media type in released
metadata. The authoritative bytes remain encrypted and bound to the CLI application
identity; downloads still pass through the normal filesystem policy boundary.

### 4. Attach to durable context

```bash
colossus --config .colossus/config.yaml run --resume \
  "Continue with the next step"
```

Use `--session SESSION_ID` instead when the exact session matters.

## Expected result

The command returns one final response, records the run in a durable session, and appends
provider and effect lifecycle evidence to the hash-chained journal. Configured protected
storage encrypts its payloads.

## Verification

```bash
colossus --config .colossus/config.yaml sessions list
colossus --config .colossus/config.yaml telemetry runs
colossus --config .colossus/config.yaml audit verify
```

Confirm that the session and run appear and the journal verifies.

## Failure path

- **Tool is missing:** run `config effective` and resolve its selection or prerequisite.
- **Request needs approval:** noninteractive runs default to `deny`; use the terminal UI
  for human approval or an explicitly reviewed approval mode.
- **Shell tool is missing:** inspect `config effective` for the selected workspace,
  sandbox profile, resolved shell, and actor scope.
- **Policy denies the action:** changing approval mode cannot reverse a deny.
- **Provider request is unknown:** inspect provider-side state before retrying.
- **Output format is wrong:** place global `--output human|json` before `run`.

## Next step

Use the [Terminal UI](terminal-ui.md) for live approvals and queued turns, or
[Sessions and context](sessions-context.md) to manage durable history.

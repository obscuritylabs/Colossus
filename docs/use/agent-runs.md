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
colossus --config .colossus/config.yaml run \
  "Summarize this repository"
```

Interactive stdout defaults to a Markdown-capable human card. Piped or redirected stdout
defaults to stable JSON.

### 2. Set explicit bounds when needed

```bash
colossus --config .colossus/config.yaml run --max-turns 12 \
  "Inspect the problem, implement the smallest change, and verify it"
```

Use `--role ROLE` to select an operator-configured model role. The role chooses a route;
the model cannot choose an endpoint or credential.

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

### 4. Attach to durable context

```bash
colossus --config .colossus/config.yaml run --resume \
  "Continue with the next step"
```

Use `--session SESSION_ID` instead when the exact session matters.

## Expected result

The command returns one final response, records the run in a durable session, and appends
provider and effect lifecycle evidence to the encrypted journal.

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
- **Policy denies the action:** changing approval mode cannot reverse a deny.
- **Provider request is unknown:** inspect provider-side state before retrying.
- **Output format is wrong:** place global `--output human|json` before `run`.

## Next step

Use the [Terminal UI](terminal-ui.md) for live approvals and queued turns, or
[Sessions and context](sessions-context.md) to manage durable history.

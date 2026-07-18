---
title: First repository task
description: Give Colossus read-only access to one repository and complete a verifiable inspection task.
audience: user
type: tutorial
---

# First repository task

## Goal

Let Colossus inspect one repository without granting workspace writes, process execution,
or arbitrary network access.

## Prerequisites

- A connected model. See [Connect a model](connect-model.md).
- A local source repository.
- The absolute path to the repository.
- A Colossus configuration stored outside the repository or referenced by an absolute
  path.

## Steps

### 1. Add one read-only filesystem root

In `.colossus/config.yaml`, add the repository's canonical absolute path:

```yaml
sandbox:
  filesystem:
    - root: /absolute/path/to/repository
      mode: read
```

Merge this field with the existing sandbox configuration. Keep write roots,
executables, and unrelated network origins absent.

### 2. Check the effective tool surface

```bash
colossus --config /absolute/path/to/.colossus/config.yaml \
  config effective
```

Confirm that repository and filesystem read tools are active, while write and execution
capabilities remain unavailable or approval-gated without matching grants.

### 3. Start from the repository

```bash
cd /absolute/path/to/repository
colossus --config /absolute/path/to/.colossus/config.yaml run \
  "Map this repository. Name its three most important components and cite the files that support your answer. Do not change anything."
```

The process working directory selects the active workspace. It never expands the
configured sandbox root.

## Expected result

Colossus returns a concise repository map with file-backed evidence. No workspace file
changes, process launches, or additional network destinations are authorized.

## Verification

Check the repository and the audit trail:

```bash
git status --short
colossus --config /absolute/path/to/.colossus/config.yaml \
  audit show --limit 20
```

The Git status should match its state before the task. Audit evidence should show the
provider and read lifecycle without a filesystem mutation.

## Failure path

- **Repository tools are hidden:** verify the absolute read root with
  `config effective`.
- **Path is outside the workspace:** start Colossus from the intended repository and
  confirm the root is canonical.
- **The model asks to run Git:** this tutorial intentionally grants no executable.
  Ask it to use repository-context and filesystem read tools instead.
- **The answer lacks evidence:** ask for exact file paths and a bounded second pass.
- **A mutation is requested:** deny it and follow
  [Access and approvals](../admin/access-and-approvals.md) before adding a write grant.

## Next step

Open [Agent runs](../use/agent-runs.md) for one-shot work or
[Terminal UI](../use/terminal-ui.md) for an ongoing interactive session.

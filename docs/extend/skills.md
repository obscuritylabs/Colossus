---
title: Skills
description: Create, validate, install, and activate data-only agent instructions with bounded resources.
audience: developer
type: how-to
---

# Skills

## Goal

Create a reusable data-only skill, validate its identity and resources, install it
through the guarded service, and activate it for one turn.

## Prerequisites

- Skills enabled with explicit bundled, repository, and user roots.
- Approval for scaffold, write, or install mutations.
- A lowercase skill name and concise description.
- Executable behavior kept outside the skill.

## Steps

### 1. Scaffold an installed user skill

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  skills scaffold release-checklist "Review native release readiness" \
  --instructions "Verify gates, artifacts, checksums, and audit evidence." \
  --resource-dir references --resource-dir tests
```

The generated directory contains `SKILL.md` with `name` and `description` frontmatter.
Allowed optional resource directories are `references/`, `scripts/`, `assets/`,
`examples/`, and `tests/`. Files under `scripts/` remain data; activation never executes
them.

### 2. Inspect and validate

```bash
colossus --config .colossus/config.yaml skills inspect release-checklist
colossus --config .colossus/config.yaml skills validate release-checklist
```

When `SKILL.md` and optional `manifest.json` both contain identity metadata, the values
must agree.

### 3. Edit with optimistic identity

```bash
colossus --config .colossus/config.yaml skills file-read \
  release-checklist SKILL.md
colossus --config .colossus/config.yaml --approval-mode ask skills write \
  release-checklist SKILL.md "UPDATED_CONTENT" \
  --expected-sha256 SHA256_FROM_READ
```

The expected hash prevents a stale overwrite. For substantial content, use an
application caller that passes the complete string without shell quoting ambiguity.

### 4. Preview and activate

```bash
colossus --config .colossus/config.yaml skills compose \
  "Review this release" --skill release-checklist
colossus --config .colossus/config.yaml run \
  --skill release-checklist "Review this release"
```

In the terminal UI, begin a message with `@release-checklist`.

## Expected result

Composition reports the selected skill and validates required tools. Only an explicitly
active skill's full instructions enter the model context.

## Verification

```bash
colossus --config .colossus/config.yaml skills list
colossus --config .colossus/config.yaml skills resources release-checklist
```

Confirm the selected provenance and bounded regular resources. Use `skills duplicates`
to expose name collisions.

## Failure path

- **Duplicate name:** inspect precedence; user overrides work only when explicitly
  enabled.
- **Required tool missing:** resolve its access and configuration prerequisite before
  composition.
- **Resource rejected:** use a safe relative regular text file under an allowed
  directory; symlinks and oversized content fail closed.
- **The skill needs to run code:** move that capability to a verified pack or configured
  MCP server.

## Next step

Use the [extension manifest reference](../reference/extension-formats.md) for exact
metadata fields, or package executable capability with [Packs](packs.md).

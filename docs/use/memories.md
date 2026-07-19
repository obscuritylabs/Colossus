---
title: Memories
description: Store, retrieve, and retire durable non-secret context with explicit scope and lifecycle.
audience: user
type: how-to
---

# Memories

## Goal

Create a repository-scoped constraint, retrieve it by meaning, and retire it without
deleting audit history.

## Prerequisites

- An initialized configuration and canonical state.
- The repository or session identifier for a scoped memory.
- Text that contains no credential, token, private key, or other secret value.

## Steps

### 1. Create a scoped memory

```bash
colossus --config .colossus/config.yaml memories create \
  "This repository requires warnings-as-errors linting" \
  --scope repository --scope-id REPOSITORY_ID --kind constraint \
  --rationale "Recorded project convention"
```

Use `global` only for broadly applicable preferences. Session and repository scopes
require `--scope-id`.

### 2. Retrieve canonical candidates

```bash
colossus --config .colossus/config.yaml memories search "linting" \
  --repository REPOSITORY_ID
colossus --config .colossus/config.yaml memories show MEMORY_ID
```

The journal owns status, scope, expiry, and content. Search indexes return candidates;
Colossus reloads and re-filters canonical records before release.

### 3. Replace or retire stale guidance

```bash
colossus --config .colossus/config.yaml memories supersede MEMORY_ID \
  "This repository runs warnings-as-errors linting in CI" \
  --rationale "Clarified when the rule applies"
```

Or archive it:

```bash
colossus --config .colossus/config.yaml memories archive MEMORY_ID
```

### 4. Check index health

```bash
colossus --config .colossus/config.yaml memories index status
```

`sync` retries queued index work. `rebuild` recreates disposable search state from the
canonical journal.

## Expected result

Active, in-scope memory is available as non-instructional background for later turns.
Superseded and archived records remain auditable but no longer steer new work.

## Verification

Run the same scoped search after superseding or archiving. Confirm that only the current
active record is released and that `memories show` preserves lineage.

## Failure path

- **No result appears:** verify scope, status, expiry, and repository identity.
- **Index is unavailable:** canonical records remain intact; sync or rebuild the
  disposable index.
- **A memory contains a secret:** rotate the credential, archive the memory, and follow
  your incident process. Memory text is not a secret store.
- **Two memories conflict:** supersede the outdated record so active context has one
  owner.

## Next step

Use [Deep research](deep-research.md) for source-backed evidence that should
remain attached to a research run instead of general memory.

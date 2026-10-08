# Preserve working memory and evidence

Use scratch for review state, sanitized evidence, and handoffs. Inspect source from the
actual target root so paths/configuration are correct. Authorized validation belongs in
an isolated fixture. Scratch supports memory; it does not sandbox or encrypt content.

## Initialize before large reads

Prefer the primary checkout's already ignored `.local/security-reviews/<review-id>/`.
In worktrees, reuse primary-checkout notes only if that path is authorized; separately
record the inspected worktree. Do not change Git configuration or add broad ignore rules
to obtain a directory name.

The stdlib-only helper is `scripts/init_review.py`, relative to this skill. Locate its
installed path with available resource tools, then run via ordinary permitted process
tools and the configured Python interpreter, for example:

```text
python3 <absolute skill root>/scripts/init_review.py --workspace <absolute checkout>
```

Quote actual paths for the host shell; placeholders are not literal commands. Python
3.10+ and normal file/process authority are required. The helper:

- Uses checkout-local storage only when Git confirms ignore status and scratch parents
  are ordinary directories, not links or redirected paths.
- Otherwise uses a unique OS temporary directory; report the location and that cleanup
  may remove it. Do not promise temporary evidence survives restart.
- Exclusively creates a review directory, owner-only POSIX directory/file modes,
  `brief.md`, `threat-model.md`, `checkpoint.md`, `coverage.json`, `findings.json`,
  and a worker directory. On Windows, host ACLs apply.
- Prints location/status JSON only. It does not inspect source, execute the target,
  install tools, change ignores/configuration, or overwrite an existing review.

If process/Python access is unavailable, use permitted filesystem tools after checking
ignore status, containment, and links. If writes are denied, use permitted durable
session/task/plan records and a bounded handoff. Do not route denied writes through
another executor or an unauthorized location. The helper grants no authority.

## A small set of owned artifacts

| Artifact | Owner and content |
| --- | --- |
| `brief.md` | Coordinator: exact scope, target snapshot/dirty state, roots, budgets, deployment assumptions, exclusions, tools/execution limits. |
| `threat-model.md` | Coordinator: source-backed boundaries, actors/assets, effective controls/resources, supplied model reference, packet questions. |
| `checkpoint.md` | Coordinator: next actions, packet IDs/states, job IDs/statuses, candidate dispositions, proof gaps, changes/evidence pointers. |
| `coverage.json` | Coordinator: reviewed/partial/unreviewed/excluded surfaces with paths, controls, and reasons; initially `not_started`. |
| `findings.json` | Coordinator: candidate records, including rejected/needs-evidence leads. Candidates are not all vulnerabilities. |
| `workers/<packet-id>.md` | Assigned worker only: bounded evidence, counterevidence, exact reviewed/remaining scope; parent may save a returned result after completion. |
| `evidence/` | Create for useful sanitized analyzer excerpts, synthetic fixtures, or observed tests. Keep bulk logs out of context. |
| `report.md` | Coordinator: final report, created on completion or explicitly labeled partial. |

For small reviews, combine brief/model/checkpoint into one note. Do not create an empty
report and call it complete. Use atomic replacement where supported; never let workers
append concurrently to shared ledgers. Use distinct safe packet IDs.

For large reviews, keep structured aggregates on disk and query/update only active
candidate or packet IDs with permitted local tools. Do not reload the full findings or
coverage file into model context for each decision. Checkpoint an index of evidence
paths and preserve original records when merging updates; file-backed memory helps
only when retrieval stays bounded.

Checkpoint after mapping, each return and validation decision, and before large output,
compaction, or stopping. Save concise evidence and actionable next steps, not transcripts
or hidden reasoning. Never persist credentials, literal secret-bearing configuration,
user records, or raw private service responses.

## Manage context proactively

Use `context.show({})` when offered at startup, before large reads/waves, and after
substantial results. Compare prepared estimates with effective input budget, not the
advertised context window. These are estimates; tool/instruction overhead can change.

As a conservative heuristic, checkpoint and split upcoming work around 60% of the
input budget. Around 75%, or earlier if a large read is next or the runtime reports
pressure, stop bulk ingestion, checkpoint, and use `context.compact({})` when offered
and authorized. Respect earlier runtime thresholds/user limits. These percentages do
not change runtime configuration or guarantee fit. Repeated compaction will not fix a
single oversized turn; reduce excerpts/output, task scope, or returned worker content.

Without telemetry, use bounded batches and checkpoint between subsystems. Do not load
all references/source/scanner logs/worker transcripts. Read evidence by focused paths
and line ranges. Keep the active question, trace, key controls, and next action in context.

## Resume from evidence

Read brief/checkpoint first, then active packet/model rows/candidate evidence. Inspect
durable job states before replacing pending workers. Do not duplicate queued/running
jobs or assume interrupted jobs had no effects. Do not restore an older snapshot merely
to avoid inspecting current evidence.

Compare revision/dirty state to the brief. Revalidate affected anchors/controls if source
changed; historical notes do not prove the current target. Verify material worker claims
against source. Retain confirmed/rejected/needs-evidence distinctions through handoffs.
Report scratch/report locations and persistence limits. Do not automatically delete
evidence, publish notes, or promote task-local findings to cross-session memory.

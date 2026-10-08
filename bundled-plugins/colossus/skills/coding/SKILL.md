---
name: coding
description: Implement features, fix bugs, debug failures, or refactor software in an existing repository or a new project. Use when a coding request needs a working change and verification of the requested behavior.
---
# Coding

Complete the requested change in the selected workspace through implementation and
verification. Scale the workflow to the task; a small fix does not need a formal plan.

Follow the user's scope, applicable repository instructions, and environment guidance.
Use only tools exposed by the runtime and respect policy and approval decisions. This
skill supplies a coding workflow, not additional tool or mutation authority.

## Understand the change

- Inspect the working tree and preserve existing edits. Read the repository's agent
  instructions, then locate the owning code, relevant tests, dependency manifests,
  lockfiles, and required checks. For a new project, choose a minimal structure suited
  to the request and the available toolchain.
- Establish the expected behavior and what would prove it works. For a bug, reproduce
  the failure or trace the smallest failing path before editing. Separate observations
  from assumptions.
- Use a short plan for work with several dependent steps. Ask a focused question when
  a missing requirement materially changes the result; continue independent work while
  waiting. Resolve routine implementation choices from repository evidence.

## Implement

- Follow existing architecture, public contracts, and code conventions. Reuse the
  project's components and libraries before adding dependencies or abstractions.
- Make the smallest coherent change that addresses the cause. Handle relevant error
  paths, input boundaries, and compatibility requirements; keep unrelated cleanup out
  of the diff.
- Add or update behavior-focused tests where they provide useful regression evidence.
  For a bug, verify that the regression test fails for the original reason and passes
  with the fix when feasible. Avoid tests that merely repeat the implementation or
  assert incidental formatting.
- Update affected usage documentation, configuration examples, and generated artifacts
  when the change requires them. Use the repository's generators rather than editing
  generated output by hand.

## Work within the environment

Environment-specific instructions own package registry locations, documentation sources,
toolchain setup, and network constraints. Follow those instructions alongside this
workflow; do not assume an air-gapped deployment lacks an internal registry or docs.

- Use the declared package manager, available tools, and locked dependencies. Obtain
  missing packages only through configured, permitted sources. Do not silently add an
  external service or credential dependency to an offline workflow.
- Consult repository source, local documentation, and configured internal documentation
  when search tools are unavailable. Do not invent registry URLs, documentation paths,
  or version-specific API behavior.
- If a needed dependency, tool, or service is unavailable, identify the exact blocker
  and continue useful local work. Keep checks requiring that prerequisite explicitly
  unverified instead of weakening them to obtain a pass.

## Verify

- Run focused checks while iterating, then the repository's required completion gates.
  Select relevant behavior tests, formatting, linting, type checks, and build checks
  from the repository's instructions and task scope.
- Exercise the changed user-visible path when applicable. A successful compilation
  alone does not prove a command, screen, or integration behaves as requested.
- Investigate failures and distinguish introduced regressions from existing failures
  or unavailable prerequisites. Fix failures caused by the change; do not suppress
  errors, remove meaningful assertions, or report an unrun check as passing.
- Review the final diff for unintended changes, temporary debugging code, sensitive
  data, and missing companion updates. Confirm the result against the requested
  behavior and the actual check output.

## Hand off

Summarize the changed behavior, relevant verification results, and any remaining
limitations or required user steps. State clearly which checks passed, failed, or could
not run. Do not claim the change is deployed, published, or fully verified without
evidence.

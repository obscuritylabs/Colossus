---
status: current
replacement: Public documentation is linked in the inventory below.
---

# Internal documentation archive

This directory contains maintainer specifications, reconstruction history, parity
evidence, acceptance records, and release procedure. It is repository-only: the public
Zensical configuration, navigation, build, search, and sitemap exclude
`internal/documentation/`.

Public documentation describes the current supported product by reader outcome.
Historical or acceptance-oriented documents remain here so maintainers can trace why a
contract exists without placing reconstruction history in a user's journey.

## Inventory

| File | Status | Purpose | Public replacement |
| --- | --- | --- | --- |
| `feature-inventory.md` | `archived` | Original product requirements and reconstruction specification; facts must be revalidated against current code before reuse | `/`, `/get-started/core-concepts/`, `/use/`, `/admin/`, `/reference/`, `/develop/architecture/` |
| `rust-reconstruction.md` | `archived` | Reconstruction implementation status and handoff history | `/develop/architecture/`, `/develop/runtime-ports/`, `/develop/state-recovery/` |
| `rust-acceptance-matrix.md` | `current` | Maintainer acceptance inventory; executable tests remain authoritative | Current executable tests and `/develop/setup-testing/` |
| `terminal-ux-parity.md` | `archived` | Presentation parity narrative and acceptance evidence | `/use/terminal-ui/`, `/reference/tui/`, `/develop/extensions-presentation/` |
| `release-process.md` | `current` | Release-maintainer procedure; intentionally not a public user guide | Root `CHANGELOG.md` for release history and GitHub Releases for artifacts |

## Status meanings

- `current` means maintainers may follow the document as an internal procedure, subject
  to the repository's executable tests and workflows.
- `archived` means historical evidence only. It is not a current product contract and
  must not be cited as one without revalidation.

## Maintenance rules

1. Keep the status and public replacement at the top of every archived document.
2. Do not link public pages into this directory.
3. Do not add this directory to `zensical.toml` or copy it into the site output.
4. Treat root `CHANGELOG.md` as the single release-history authority.
5. Prefer executable tests, generated CLI help, parser-backed examples, and current
   reference pages over historical prose when resolving conflicts.

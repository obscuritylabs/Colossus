# Release Process

This process keeps Colossus releases reproducible, documented, and suitable for offline
review.

## Release readiness

Before tagging a release:

1. Confirm the working tree contains only intentional changes.
2. Update `CHANGELOG.md` with user-facing changes, security notes, and migration steps.
3. Update the version in `pyproject.toml`.
4. Review `README.md`, `docs/`, `SECURITY.md`, and bundle format notes for accuracy.
5. Confirm `docs/TOOLS.md`, built-in `ToolSpec` schemas, bundled skill `required_tools`,
   and tool tests describe the same catalog.
6. Run the verification commands:

```bash
uv run pytest
uv run ruff check .
uv run mypy src/colossus
uv run python -m build
```

## Artifact review

Inspect the generated artifacts:

```bash
python -m tarfile -l dist/colossus-*.tar.gz
python -m zipfile -l dist/colossus-*-py3-none-any.whl
```

Confirm that the source distribution includes:

- `src/`
- `tests/`
- `docs/`
- `README.md`
- `LICENSE`
- `CHANGELOG.md`
- `SECURITY.md`
- `AGENTS.md`
- `pyproject.toml`

## Offline bundle preparation

For airgapped releases, prepare a bundle directory with:

- Built Colossus wheel and source distribution.
- Dependency wheelhouse for the target platform.
- `uv.lock`.
- SBOM and signature material.
- Reviewed skills and manifests.
- Bundle `manifest.json` with SHA-256 checksums.

Verify it before distribution:

```bash
uv run colossus bundle verify ./bundle
```

## Tagging

Use an annotated tag after CI passes:

```bash
git tag -a v0.1.0 -m "Colossus v0.1.0"
git push origin v0.1.0
```

Attach release artifacts, checksums, SBOM, signatures, and the relevant changelog
section to the release record.

## Post-release

- Confirm package metadata renders correctly.
- Confirm release artifacts can be installed in a clean Python 3.12 environment.
- Confirm offline bundle verification succeeds from the published artifacts.
- Open follow-up issues for any deferred hardening or documentation gaps.

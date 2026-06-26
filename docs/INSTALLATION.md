# Installation

Colossus targets Python 3.12 and uses `uv` for reproducible local development.

## From a source checkout

```bash
uv sync --extra dev
uv run colossus --help
uv run colossus run "hello"
```

The default `echo` provider does not require credentials or network access. It is useful
for validating the CLI, audit path, skill loading, and orchestration loop in a fresh
checkout.

## Development environment

Install development dependencies and run the release readiness checks:

```bash
uv sync --extra dev
./scripts/install-git-hooks.sh
uv run pytest
uv run ruff check .
uv run mypy src/colossus
uv run python -m build
```

Generated wheels and source distributions are written to `dist/`.

## Installed command

The package exposes the `colossus` console script:

```bash
colossus run "hello"
colossus repl
colossus config init
colossus skills list
colossus tools list
colossus bundle verify ./bundle
```

When running from a checkout, prefix commands with `uv run`.

## Platform paths

Colossus uses platform-specific user directories through `platformdirs`:

- Config file: `config.json` under the user config directory for app name `colossus`.
- Runtime data: state, audit logs, and user skills under the user data directory for
  app name `colossus`.

Run `uv run colossus config init` to create the config file at the exact path for the
current platform, then `uv run colossus config show` to inspect the resolved values.

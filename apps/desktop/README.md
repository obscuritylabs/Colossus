# Colossus Desktop

Tauri 2 desktop client for Colossus. The UI is React and Vite; the native layer is
Rust. Desktop normally supervises a bundled Managed Local sidecar and can also connect
to an explicitly enrolled External worker.

This README is for contributors. For installation and operator setup, see
[Colossus Desktop](../../docs/get-started/desktop.md) and the
[Windows Desktop Developer Preview](../../docs/get-started/windows-desktop.md).

## Run the app

From the repository root:

```bash
./scripts/desktop-dev
```

On Windows:

```powershell
powershell -File .\scripts\desktop-dev.ps1
```

The launcher installs the locked renderer dependencies, builds and stages the native
CLI and sidecar, and starts Tauri. A separate daemon is not required for Managed Local.

Desktop requires Node.js 22.12 or newer, Rust 1.96, and the platform dependencies
listed in [Development setup and testing](../../docs/develop/setup-testing.md).

## Run the renderer with fixtures

Use the development fixtures for UI work that does not require native commands:

```bash
cd apps/desktop
npm ci --ignore-scripts
npm run dev -- --host 127.0.0.1
```

Open one of these routes:

| Route                            | Purpose                                  |
| -------------------------------- | ---------------------------------------- |
| `/?fixture=operations-studio`    | Main completed-session workspace         |
| `/?fixture=interaction-question` | Pending question and response flow       |
| `/?fixture=plan-workflow`        | Completed Plan Mode run and plan actions |
| `/?fixture=activity-comparison`  | Session activity presentation states     |

Fixtures are development-only. Production builds use the native bridge.

## Project layout

| Path                        | Responsibility                                                            |
| --------------------------- | ------------------------------------------------------------------------- |
| `src/`                      | React renderer, state, presenters, and components                         |
| `src/dev/`                  | Deterministic development fixtures                                        |
| `tests/browser/`            | Playwright interaction and accessibility tests                            |
| `src-tauri/src/`            | Native commands, sidecar lifecycle, credentials, and workspace validation |
| `src-tauri/capabilities/`   | Tauri capability declarations                                             |
| `src-tauri/connection.json` | Non-secret template for an enrolled External target                       |

The renderer is an interface only. Model, tool, policy, sandbox, and canonical-state
logic belongs in the Rust application and runtime crates.

## UI foundations

Desktop appearance is owned by `src/theme/`:

- `theme.css` defines color, typography, spacing, radius, control, and settings-layout
  tokens. Dark and light palettes override semantic tokens rather than component rules.
- `appearance.ts` owns the versioned device-local preference contract and safe parsing.
- `AppearanceProvider.tsx` resolves System, Dark, and Light color modes, applies Compact,
  Comfortable, or Large text sizing, and reacts to operating-system theme changes.

New components should use semantic tokens such as `--surface-panel`, `--text-strong`,
`--font-size-body`, and `--control-height`; do not add palette-specific hex values or
one-off font sizes. New settings tabs should render one `.managed-settings-body` and let
the shared `--settings-content-max` contract own width and centering. Pane-specific
maximum widths make tabs jump as users navigate and are not supported.

Use the state tokens for interactive surfaces instead of inventing component colors:
`--surface-hover`, `--surface-selected`, `--focus-ring`, and the semantic success,
warning, and danger families. Source and artifact previews use the shared `--code-*`
tokens. Syntax highlighting must request the resolved appearance theme so Shiki token
colors change with the surrounding code canvas. Visible supporting copy must be at least
the `--font-size-caption` size; denser metadata should change layout before shrinking
below that floor.

Appearance changes require browser coverage in both color themes, all three text sizes,
responsive overflow checks, and hover/focus coverage for interactive rows. Include an
Axe scan when adding visible states. The Desktop contract suite rejects fixed component
background colors and typography smaller than the caption token.

## Checks

Renderer checks:

```bash
cd apps/desktop
npm run check
npm run test:browser
npm run build
```

Native Desktop checks from the repository root:

```bash
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml \
  --all-targets -- -D warnings
```

Before opening a PR, run the repository's change-selected gate:

```bash
cargo xtask pr --base origin/main
```

See [Testing strategy](../../docs/develop/testing.md) for the full verification tiers.

## Native boundary

- Credentials and private runtime paths stay in native Rust and are not returned to
  the renderer.
- Renderer access to the host is limited to registered Tauri commands and capabilities.
- Model-authored Markdown is sanitized; raw HTML, remote images, and model-authored
  navigation are disabled.
- Managed Local state uses a Desktop-specific workspace partition and does not reuse
  CLI or TUI state.

Changes to native commands, capabilities, sidecar bootstrap, credentials, terminals,
or workspace handling are security-boundary changes. Read
[Application SDK: Tauri integration](../../docs/develop/application-sdk.md#tauri-integration),
[Security architecture](../../docs/develop/security-architecture.md), and the
[Colossus home contract](../../docs/reference/colossus-home.md) before editing them.

## External targets and packaging

External worker enrollment, credential rotation, release validation, signing, and
packaging are documented elsewhere:

- [Add an External target](../../docs/get-started/desktop.md#6-add-an-external-target-when-needed)
- [Connection and enrollment](../../docs/develop/application-sdk.md#connection-and-enrollment)
- [Development and release setup](../../docs/develop/setup-testing.md)
- [CI/CD and release gates](../../docs/develop/ci-cd.md)

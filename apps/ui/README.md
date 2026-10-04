# Colossus shared UI

`@colossus/ui` is a private source package consumed by Desktop and the VS Code
webviews from the same checkout. It owns the theme tokens (Colossus blue, light,
neutral dark, and Hacker), brand mark, settings frame, composer input and controls,
composer sizing/keyboard behavior, and the accessible dropdown control. Both hosts
compile this source into their own renderer bundles; this package is not independently
published or versioned for release.

Desktop keeps Tauri commands, appearance persistence, workspace/session orchestration,
queues, plugins, and capability decisions. VS Code keeps its native extension host,
worker SDK, credential handling, validated message protocol, and view projections.
Shared components take presentation props and callbacks only. They cannot import a
host bridge or transport. A future browser frontend can consume this package while
its Python backend owns the SDK connection and authentication.

The app build and lockfiles remain separate. After changing this package, refresh
both local file dependencies with `npm ci --ignore-scripts` in `apps/desktop` and
`apps/vscode`, then run their `check`, `build`, and affected `test:browser` commands.
CI installs the shared source afresh and runs both app gates when `apps/ui` changes.
No generated UI bundle needs committing. React is pinned to the same peer version
in both consumers so each renderer has one React instance.

Neutral dark is the VS Code default. Desktop keeps Colossus blue by default and
offers neutral dark and Hacker in Global → Appearance → Dark palette. Desktop keeps saved
custom palette colors while neutral dark or Hacker is selected. Hacker translates
the TUI palette from `crates/colossus-presentation/src/palette.rs`: green prompts
and success, pale-green messages, cyan tools, amber warnings, and red errors.
The shared CSS adds near-black graphical surfaces and retains Colossus typography
and branding. It is selectable in both apps; preferences remain local to each host.
Light and high contrast keep their respective token families. The native activity bar, left workspace view, and
right chat view stay owned by VS Code.

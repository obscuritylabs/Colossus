# Documentation build inputs

This directory contains repository-only inputs for the public Zensical site. It is not
inside `docs/` and is excluded from publishing and search.

- `legacy-routes.tsv` is the canonical compatibility-route manifest.
- `tui-offline-session.txt` is the literal 132×24 Ratatui frame used for the homepage
  screenshot.

The TUI frame was recaptured on 2026-08-20 from `target/debug/colossus` in a fixed-size
tmux pane immediately after `config init`, using the generated `echo` profile, the
`workspace-development` sandbox preset, and keyless offline storage. It shows the
current launch rail and its explicit plaintext-journal posture warning before any prompt
is submitted. The frame contains no credential, repository content, or external
response. Reproduce the source frame from the repository root with an isolated
temporary config, then use:

```bash
tmux new-session -d -s colossus-doc-capture -x 132 -y 24 \
  "/absolute/path/to/colossus --config /temporary/config.yaml tui --alt-screen"
tmux capture-pane -t colossus-doc-capture -p
```

Render the checked-in frame into the public PNG with:

```bash
./scripts/render-docs-tui-capture
```

The renderer uses Pango with Menlo 17, fixed Ocean palette values, 28×32 pixel
padding, and grayscale antialiasing. Its expected output is a 1384×554 RGB PNG.
`pango-view` and the macOS system Menlo font are maintenance dependencies only; they
are not loaded by the documentation site or Colossus runtime.

The frame keeps the launch rail, effective operational settings, security posture,
composer, and status footer in one continuous image. The short UUIDv7 session prefix
records the captured local session and is not an external identifier.

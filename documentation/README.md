# Documentation build inputs

This directory contains repository-only inputs for the public Zensical site. It is not
inside `docs/` and is excluded from publishing and search.

- `legacy-routes.tsv` is the canonical compatibility-route manifest.
- `tui-offline-session.txt` is the literal 112×34 Ratatui frame used for the homepage
  screenshot.

The TUI frame was captured on 2026-07-18 from `target/debug/colossus` in a fixed-size
tmux pane after `config init`, using the generated `echo` profile. The exact prompt was:

```text
Summarize what Colossus controls before a tool effect.
```

The response is deterministic because the echo provider returns the prompt. The frame
contains no credential, filesystem path, repository content, or external response.
Reproduce the source frame from the repository root with an isolated temporary config,
then use:

```bash
tmux new-session -d -s colossus-doc-capture -x 112 -y 34 \
  "/absolute/path/to/colossus --config /temporary/config.yaml tui"
tmux send-keys -t colossus-doc-capture \
  "Summarize what Colossus controls before a tool effect." Enter
tmux capture-pane -t colossus-doc-capture -p
```

The generated PNG is a direct monospace rendering of those 34 rows using the built-in
default palette. The short UUIDv7 session prefix records the captured run and is not an
external identifier.

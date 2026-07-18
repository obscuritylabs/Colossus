# Contributing

## Commit Messages

Colossus uses Conventional Commits for new commits:

```text
<type>[optional scope][!]: <description>
```

Examples:

```text
feat(tui): add approval prompt cleanup
fix: handle invalid tool arguments as recoverable errors
docs: explain structured shell usage
```

Allowed types are `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`,
`revert`, `security`, `style`, and `test`.

Install the local commit hook after cloning:

```bash
./scripts/install-git-hooks.sh
```

The hook validates commit messages with:

```bash
./scripts/check_conventional_commit.sh .git/COMMIT_EDITMSG
```

The checker is POSIX shell and does not require Python. CI validates pull request titles
and pushed commit messages with the same script.

## Use The Local Compilation Cache

Colossus supports an opt-in local `sccache` wrapper for cold builds and development
across multiple worktrees. On macOS, install the Homebrew bottle:

```bash
brew install sccache
```

Prefix any Cargo command with the repository wrapper:

```bash
./scripts/cargo-sccache check -p colossus-runtime
./scripts/cargo-sccache test-fast
sccache --show-stats
```

The wrapper sets `RUSTC_WRAPPER=sccache`, disables rustc incremental compilation so
results are cacheable, normalizes every currently registered Git worktree through
`SCCACHE_BASEDIRS`, and bounds the local cache to 5 GiB. Restart the sccache server after
adding a new worktree so it receives the updated base-directory list:

```bash
sccache --stop-server
```

Override `CARGO_INCREMENTAL`, `SCCACHE_BASEDIRS`, or `SCCACHE_CACHE_SIZE` explicitly when
another tradeoff is desired. Run `cargo` directly when repeated edits in one already-warm
worktree benefit more from rustc's worktree-local incremental cache. The cache accelerates
cacheable library compilation; Rust binaries and end-to-end test execution still require
linking and execution.

## Use Verification Tiers

Use the narrowest tier that covers the current edit:

```bash
# One changed crate, optionally followed by a directly affected integration test.
cargo test -p colossus-policy --lib
cargo test -p colossus-cli --test config_security

# All workspace library tests; excludes the long CLI acceptance scenarios.
cargo test-fast

# Complete workspace suite, including end-to-end acceptance scenarios.
cargo test-full
```

`cargo test-fast` is the normal workspace-wide iteration tier. Before declaring an
implementation complete, run the authoritative formatting, Clippy, and full workspace
test gates documented in the root `AGENTS.md`; the fast tier never replaces them.
Structural changes must also keep crate roots as API/composition surfaces:

```bash
./scripts/check_crate_roots.sh
```

## Run A Development TUI

Use the repository launcher when iterating on the debug binary:

```bash
./scripts/colossus-dev --approval-mode full-access tui
```

The launcher keeps development keys, redb state, and the secure anchor isolated under
`.colossus` and does not access the platform credential store. See
[Configuration](CONFIGURATION.md#isolated-source-development) for the generated files
and direct initialization command.

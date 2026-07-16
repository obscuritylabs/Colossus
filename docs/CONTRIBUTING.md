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

## Run A Development TUI

Use the repository launcher when iterating on the debug binary:

```bash
./scripts/colossus-dev --approval-mode full-access tui
```

The launcher keeps development keys, redb state, and the secure anchor isolated under
`.colossus` and does not access the platform credential store. See
[Configuration](CONFIGURATION.md#isolated-source-development) for the generated files
and direct initialization command.

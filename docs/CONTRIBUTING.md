# Contributing

## Commit Messages

Colossus uses Conventional Commits for new commits:

```text
<type>[optional scope][!]: <description>
```

Examples:

```text
feat(repl): add approval prompt cleanup
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

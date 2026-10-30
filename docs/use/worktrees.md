---
title: Git worktrees
description: Create and open independent Git checkouts from Desktop or the CLI.
audience: user
type: how-to
---

# Git worktrees

A worktree gives a branch its own checkout. Use it when you want to work on another
change without moving the files or branch in your current workspace. Colossus Desktop
and the CLI share a private inventory of the worktrees they create.

## Desktop

Select a local Git repository, then open **Settings → Global → Worktrees**. Allow access to its
shared Git metadata when prompted. Enter a short lowercase name, such as `fix-login`,
and choose **Create worktree**. The default creates branch `colossus/fix-login` from
`HEAD` and opens the new checkout as a Desktop workspace.

You can choose another local branch, tag or full commit ID as the base. To check out
an existing branch, select **Use an existing local branch** and enter its name.
Colossus does not fetch remote refs or overwrite existing branches.

The defaults let you change the branch prefix, turn off opening after creation, or
select a parent folder for a dedicated `ColossusWorktrees` directory. Choosing another
directory does not move existing checkouts. Select their original parent again to
manage them there.

## CLI

These commands inspect and change worktrees without starting a runtime, loading model
credentials or contacting a provider. Run them from the repository root, or supply
the root with `--workspace`:

```sh
colossus --workspace /path/to/repository worktree list
colossus --workspace /path/to/repository worktree create fix-login
colossus --workspace /path/to/repository worktree create review --branch review-ready --existing-branch
colossus --workspace /path/to/repository worktree create experiment --base main
```

Copy a worktree's `id` from the inventory. `open` restores an archived worktree if
necessary and starts the ordinary TUI in its checkout. The TUI uses the usual local
configuration and provider setup. `--print-path` returns the verified path and exits:

```sh
colossus worktree open WORKTREE_ID
colossus worktree open WORKTREE_ID --print-path
colossus worktree archive WORKTREE_ID
colossus worktree remove WORKTREE_ID --yes
```

Creation prints the new checkout's metadata; the Desktop open-after-create preference
does not start a TUI from the CLI's `create` command.

```sh
colossus worktree settings
colossus worktree settings --branch-prefix feature/ --open-after-create false
colossus worktree settings --storage-directory /path/to/new-private-worktrees
colossus worktree settings --reset-directory
colossus worktree --directory /path/to/another-owned-inventory list
```

The default directory is `worktrees` beneath an explicit `COLOSSUS_HOME`. Otherwise,
it is `ColossusWorktrees` in the current user's local application-data directory.
Desktop and CLI use the same default. The inventory is limited to 128 live entries.

## Archive and remove

Archive preserves the checkout's files and Git branch. Close Desktop workspaces,
workers, terminals and CLI sessions using it first. Colossus processes hold native
file locks for their working lifetime; another process cannot archive or remove a
checkout while one of those shared-use locks is held. Sessions opened in a subfolder
also lease every enclosing managed checkout, including when the subfolder contains
another repository. Archived checkouts must be restored before a session can start
in any of their subfolders.

Removal requires an archived, unused, unlocked checkout with no local changes,
untracked files or ignored files. Remove build output yourself only when you no
longer need it. There is no force-removal option. Removal retains the Git branch and
a local receipt. Colossus never removes worktrees created by another application.

An interrupted create or remove remains visible with its state and a note. Refresh
after an uncertain result instead of repeating the action. Colossus does not
automatically finish interrupted deletion or discard files.

## Repository requirements

Use a conventional repository root or a linked worktree with its normal Git backlink.
Managed worktrees currently reject Git configuration includes, external attribute or
ignore files, alternate object stores, symlink/reparse-point parent paths and submodules.
A tracked
relative file link is supported when it stays inside the checkout and names a regular
tracked file; link chains, directory links and escaping targets are refused. Windows
Git may materialize these links as ordinary files containing the relative target.
Large repositories can exceed bounded inspection
limits. Git hooks, shell commands, credential helpers and network operations are not
used by this feature. Existing read-only Git inspection remains available separately.

Independent applications that ignore Colossus leases can still change files during
inspection. Removal is based on a fresh clean HEAD/index check and checked private
paths; it does not atomically exclude those external same-user writers.

---
title: Workspaces
description: Add, switch, search, and archive Desktop workspaces without losing their threads.
audience: user
type: how-to
icon: lucide/folders
---

# Workspaces

A Desktop workspace is a folder selected through the native picker. Its name appears in the left sidebar, followed by its threads and runtime state. Managed Local gives each workspace its own configuration and private state partition. It can keep up to four folder-backed workspaces live at once; others may sleep until selected.

## Add and switch workspaces

Open **Manage Workspaces → Add Workspace from folder**, choose a folder you own, and complete its provider, model, and access setup. Select a workspace name in the sidebar to make it the context for new work. **Connections** lists the folder-backed runtimes and their states if you want a wider view.

The selected workspace determines the working directory for tools, file browsing, Git inspection, and local terminal sessions. A folder alone does not limit every authorized effect to that path; choose the [execution boundary](settings.md#access-and-execution-boundaries) that fits the project. Desktop keeps CLI/TUI and Desktop state separate even if both use the same repository.

## Find and organize work

Use **Search threads** in the sidebar. **This Workspace** narrows results to the selected folder; **All Workspaces** searches across your Desktop workspaces. Enable **Include archived Workspaces and threads** when searching older work. Select a result to open it, or restore it if archived.

Each workspace row can start a new thread, expand or collapse its threads, and open a menu for renaming or archiving. Thread menus also support rename, pin or unpin, and archive. Pinned threads are grouped for quick access. Archived items are kept in Desktop state and can be restored from search or workspace management; archiving is different from deleting the project folder.

Switching workspaces closes that workspace's open local terminal sessions and changes which managed runtime receives new Work actions. A run already in progress keeps the configuration revision it accepted when it started. If you apply settings during active work, Desktop waits for that work to drain before restarting the workspace.

## Understand runtime states

The workspace row and **Connections** show states such as **Ready**, **Starting**, **Restarting**, **Stopping**, **Sleeping**, or **Failed**. **Ready** means its Managed Local worker is connected; the Work header also shows whether the agent is online. If a workspace fails to start, open [Settings and diagnostics](settings.md#desktop-preferences-and-support), check the provider or local runtime status, and retry after resolving the reported issue.

An installed worker already using the folder cannot be silently replaced by Managed Local. Connect that worker as an [External target](external-targets.md) instead.

## Next step

[Start a thread](work.md#start-a-task) in the selected workspace, then use [Session views](session-views.md) to inspect its durable records.

---
title: Work and threads
description: Start Desktop runs, guide work in progress, and return to earlier conversations.
audience: user
type: how-to
icon: lucide/messages-square
---

# Work and threads

**Work** is the center of Desktop. Select a workspace in the left sidebar, choose **New thread**, and give Colossus a task. The composer shows the selected model, permission mode, and run mode before you send.

## Start a task

Choose a mode beside the composer:

| Mode | Use it when |
| --- | --- |
| **Plan** | You want a proposed sequence to inspect and approve before execution. |
| **Execute** | You want Colossus to carry out an authorized task, such as exploring or changing a repository. |
| **Research** | You want a source-backed research run. Availability depends on the selected runtime and configuration. |

Write a concrete request, including any limits that matter, and press **Enter** or the send button. **Shift+Enter** adds a new line. You can attach a supported PNG, JPEG, WebP, UTF-8 text, or source file with the paperclip. Desktop shows a thread in the selected workspace as soon as the run starts. Tool effects still follow that workspace's [access and approval settings](settings.md#access-and-execution-boundaries).

For a first task, try asking the agent to explain the repository without changing files. To inspect its response and evidence, open [Session views](session-views.md).

## Guide a run in progress

When Colossus is working, you can type another message without losing your draft:

- **Queue** puts the follow-up in **Next up** after the current response.
- **Redirect** stops the current response and sends your new guidance next.
- **Stop** cancels the current response and pauses queued messages. Your unsent draft remains. Choose **Resume queue** when you want the queued work to continue.

The queue and its pause are part of the current Desktop session. For work that must survive an interruption, use a durable [plan or task](../use/tasks-decisions-plans.md) and return to its session later.

The permission selector beside the composer controls what happens when an effect needs approval. **Ask** shows an approval card. **Deny** fails such effects closed. **Risk auto** evaluates eligible low-risk effects, while **Full access** satisfies approval obligations without prompting. Expanding authority requires a native operating-system confirmation. This selector does not change the workspace's access profile or execution boundary. [See the full access model](settings.md#approval-mode).

## Return to a conversation

Select a thread in the sidebar to read its released messages and continue it. Use **Search threads** to find a conversation in **This Workspace** or **All Workspaces**. The search can include archived workspaces and threads when you enable that option. **Load older threads** retrieves more history as needed.

Open a thread's **More actions** menu to rename, pin or unpin, and archive it. Pinned threads stay easy to reach; archived threads remain searchable and can be restored. You can also rename or archive a workspace from its menu, then restore it from **Manage Workspaces**. Switching workspaces changes the active managed runtime and closes local terminal sessions tied to the previous workspace.

## Use a Desktop command

Type `/` at the start of the composer to open Desktop's command menu. Use **Up/Down** to choose a suggestion, **Tab** to complete it, **Escape** to close the menu, and **Enter** to run a complete command. Commands such as `/plan`, `/execute`, `/research`, `/new`, `/resume`, `/work`, `/agents`, `/artifacts`, `/connections`, `/settings`, and `/tui` navigate or change Desktop's local UI state. Desktop handles them itself; it does not send an unknown slash command to the model.

The bundled [Terminal UI](../use/terminal-ui.md) has a larger command set. Its plan approval and execution commands are not substitutes for the authenticated Plan actions in Desktop.

## Next step

Use [Session views](session-views.md) to inspect what happened, or open [Tools](tools.md) to work with files and outputs beside the conversation.

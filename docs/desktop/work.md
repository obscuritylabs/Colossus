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

Write a concrete request, including any limits that matter, and press **Enter** or the send button. **Shift+Enter** adds a new line. You can attach a supported PNG, JPEG, WebP, UTF-8 text, or source file with the paperclip. Native PDF and Office documents require a model with document inputs enabled and a compatible provider API; see [formats and limits](../reference/configuration/providers-models.md#native-document-inputs). The provider interprets documents; Desktop does not run a local document parser. Desktop shows a thread in the selected workspace as soon as the run starts. Tool effects still follow that workspace's [access and approval settings](settings.md#access-and-execution-boundaries).

Pasting more than 1,000 characters into the message box shows a short **Pasted Content** placeholder, as in the TUI. The full text remains part of the draft, counts toward the prompt size limit, and is sent in place of the placeholder. You can write before or after it or paste more text. Deleting or changing a placeholder removes its hidden text from the draft.

For a first task, try asking the agent to explain the repository without changing files. To inspect its response and evidence, open [Session views](session-views.md).

## Capture another Windows app

Open **Settings → Global → Composer & companion**, enable **App Shots**, and review
the native confirmation. With the app you want to share in front, press
**Control+Alt+Shift+S**. Desktop opens a preview of that window’s screenshot and any
available visible accessibility text. Expand **View captured text** to inspect it;
clear **Include captured text** to attach only the image. Password-marked text fields
are excluded, but the screenshot can still contain anything visible in the window.

Choose **Attach to current draft**, add your message, and send when ready. **Discard**
or **Escape** closes the preview. Switching chats or workspaces discards the pending
capture. Captures stay in memory for up to ten minutes until you attach or discard
them; attaching uploads the selected parts to that chat’s runtime as ordinary input
artifacts. Nothing is sent to a model until you send the message.

App Shots start off. Change the shortcut in the same settings page if it conflicts
with another app. Capture supports visible ordinary-user windows on Windows; elevated,
protected, minimized, or unavailable windows may refuse capture. Accessibility text is
limited to visible labels exposed by the app, so some captures have only an image.
This shortcut does not enable agent computer control.

## Suggest the next prompt

After a completed response, Desktop can offer a next step above an empty composer.
Press **Tab** while the editor is focused, or click the suggestion, to insert an
editable draft. **Escape** dismisses it. Sending always takes a separate action.

Choose **Off**, **Local**, or **Model** under **Settings → Global → Composer & companion**.
**Local** is the default and extracts an offered next action from the completed
response without another provider call; some responses have no suggestion. **Model**
uses the completed run’s model and bounded recent released user/assistant messages
for one short prediction. It uses additional provider tokens, shown separately as
**Prediction** usage. Tools and system messages are excluded. Unavailable predictions
fall back to Local. Switching context, editing the draft, or starting another run
hides stale suggestions.

## Choose a model and reasoning level

For a managed local workspace, select the model bubble above your draft. Search the
configured models by model, provider, or profile, then select a model. The same popover
keeps a reasoning slider below the list; use the arrow keys to choose a level, or return
to **Model default**. **Done** or **Escape** closes the picker and keeps your draft.

The choice applies to later messages in the current conversation while Desktop is open.
Changing models resets reasoning to that model's saved default. A queued message keeps
the choices captured when you queued it, and an active response keeps its original
model. **Role default** restores workspace routing. **Configure models** opens the
workspace model settings. External connections and Research use their configured routes.

## Guide a run in progress

When Colossus is working, you can type another message without losing your draft:

- **Queue** puts the follow-up in **Next up** after the current response.
- **Redirect** stops the current response and sends your new guidance next.
- **Stop** cancels the current response and pauses queued messages. Your unsent draft remains. Choose **Resume queue** when you want the queued work to continue.

The queue and its pause are part of the current Desktop session. For work that must survive an interruption, use a durable [plan or task](../use/tasks-decisions-plans.md) and return to its session later.

The permission selector beside the composer controls what happens when an effect needs approval. **Ask** shows an approval card. **Deny** fails such effects closed. **Risk auto** evaluates eligible low-risk effects, while **Full access** satisfies approval obligations without prompting. Your selection applies directly between managed runs. This selector does not change the workspace's access profile or execution boundary. [See the full access model](settings.md#approval-mode).

## Return to a conversation

Select a thread in the sidebar to read its released messages and continue it. Use **Search threads** to find a conversation in **This Workspace** or **All Workspaces**. The search can include archived workspaces and threads when you enable that option. **Load older threads** retrieves more history as needed.

Open a thread's **More actions** menu to rename, pin or unpin, and archive it. Pinned threads stay easy to reach; archived threads remain searchable and can be restored. You can also rename or archive a workspace from its menu, then restore it from **Manage Workspaces**. Switching workspaces changes the active managed runtime and closes local terminal sessions tied to the previous workspace.

## Use a Desktop command

Type `/` at the start of the composer to open Desktop's command menu. Use **Up/Down** to choose a suggestion, **Tab** to complete it, **Escape** to close the menu, and **Enter** to run a complete command. Commands such as `/plan`, `/execute`, `/research`, `/new`, `/resume`, `/work`, `/agents`, `/artifacts`, `/connections`, `/settings`, and `/tui` navigate or change Desktop's local UI state. Desktop handles them itself; it does not send an unknown slash command to the model.

The bundled [Terminal UI](../use/terminal-ui.md) has a larger command set. Its plan approval and execution commands are not substitutes for the authenticated Plan actions in Desktop.

## Next step

Use [Session views](session-views.md) to inspect what happened, or open [Tools](tools.md) to work with files and outputs beside the conversation.

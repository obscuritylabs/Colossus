---
title: Work in Cloud
description: Start hosted conversations, work with connected agents, and open an enabled host shell.
audience: user
type: how-to
icon: lucide/messages-square
---

# Work in Cloud

Open your organization's Cloud URL, sign in, and choose a project. Ask your
administrator for project access if the one you need is missing.

## Find your way around

| Open | To |
| --- | --- |
| **Home** | See your projects and recent activity. |
| **Chat** | Start a personal hosted conversation, if enabled for the project. |
| **Hosted** | Inspect your personal execution target, running work, and run history. |
| **Fleet** | Find a connected host, workspace, and its conversations or shell. |
| **Tasks** | Follow execution requests and open their conversations. |
| **Analytics** | Review retained run outcomes and provider usage. Missing usage or prices remain unknown. |

## Start a hosted conversation

1. Open **Chat** and select a project.
2. Choose an available model. Connect your account in **Connections** if required.
3. Submit a message and follow the streamed response.
4. Answer prompts or exact effect approvals when requested. Use the run's cancel
   control to stop work.

Execution loads when work needs it. Reading history or choosing a model does not
keep a runtime running. Accepted work continues when you close the browser.

Attach up to four UTF-8 text, Markdown, CSV, or JSON files per submission, each no
larger than 256 KiB. They use protected account-owned storage and do not create a
filesystem or shell environment.

In **Hosted**, choose **Load runtime** to start execution; this requires Execute
access. **Release idle runtime** also requires Control and refuses active or uncertain
work. Release preserves conversations and managed files. New work loads execution again.

Personal conversations and execution belong to your account in that project.
Other users, including global administrators, cannot read or control them.

## Keep hosted chats close

Choose **Pin** in a chat's actions menu to put it in **Pinned** above recent history.
This leaves its navigation and running work unchanged. Choose **Unpin** to return it
to its date group.

Select the **Pinned** heading to collapse or expand the list. This browser remembers
your choice for the project. Search temporarily shows matching pins; clearing search
restores your saved collapse preference.

Pins stay visible while you page through older chats. Title search filters pins and
recent history together. Archiving removes a chat from the open list; restoring a
pinned chat returns it to **Pinned**. You can unpin it from the archived list too.

Each account can pin up to 32 chats per project. Pins are saved in this browser,
separately for each account and project. Clearing browser storage removes them. If
storage is unavailable, pins last only for the current browser session.

## Work with a connected agent

1. Open **Fleet**, choose a host, then a workspace.
2. Open a thread or start a conversation on that agent.
3. Submit a message and follow its task/run. Wait for the first turn to establish
   its session before sending a follow-up.

A conversation stays assigned to its original agent and workspace. Saved history
remains readable while the agent is offline; later messages queue for that same
agent. Disconnection does not move accepted work to another machine.

Rename and archive require Control access. Archive hides the conversation without
cancelling its work. History can contain an explicit output limit or incomplete
excerpt; continuation still uses the runtime's canonical session context.

Local Desktop and CLI history is private until its owner explicitly shares it.
See [conversation sharing](connect-runtimes.md#share-local-conversations).

## Find or save conversation text

In an open conversation, choose **Find in loaded conversation** and enter text.
Use the previous and next buttons to move between matching messages. Enter moves
forward; Shift+Enter moves backward. Escape closes search and returns focus to Find.
Search covers the loaded user and assistant messages. Choose **Load earlier history**
to include more of the saved conversation.

Open **Conversation actions** and choose **Copy loaded conversation** or
**Download loaded conversation** to save a Markdown snapshot. The snapshot includes
only the user and assistant text released to this view, with its author and time.
Run activity, tools, attachments, and private agent context are excluded. Unloaded
history is excluded too; the file reports incomplete history and responses still
in progress. These actions also work when viewing saved history from an offline agent.

## Open a shell on a connected host

1. Open **Fleet**, select a host, then its workspace.
2. Choose **Open Shell**. If it is absent, ask an administrator to check
   [web shell enablement](connect-runtimes.md#enable-a-web-shell) and your project's
   **Terminal operator** role.
3. Run commands in the terminal. The shell starts in the selected workspace as the
   worker's OS account, with that account's filesystem and network access. Commands
   run outside agent tool approvals and are not added to model context.

Use **Interrupt** to send an interrupt to the current process. Use **Detach** to
leave the shell running; reopening **Open Shell** reattaches while the process is
retained, for up to two minutes without an active reader. Use **Close Shell** to end
the shell, or **New Shell** to start another session within the host's limits.

After a connection loss, choose **Reconnect** when offered. It attaches to the
existing worker-owned process; it does not replay commands or resend unconfirmed
input. A worker or machine restart ends the shell. If output history has a gap, the
terminal reports it instead of reconstructing missing output.

## Share a page or return later

Copy the browser address for a project, agent, conversation, or task. The recipient
must sign in with their own existing permissions. Direct links survive sign-in;
browser Back and Forward follow your navigation history.

If a run reports an unknown outcome, inspect the original run before submitting it
again. A lost connection can leave an action completed remotely without a receipt.

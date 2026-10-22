---
title: Work in Cloud
description: Find a project, start a conversation, and follow work across hosted and connected agents.
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
| **Fleet** | Find a connected host, workspace, and its conversations. |
| **Tasks** | Follow execution requests and open their conversations. |
| **Analytics** | Review retained run outcomes and provider usage. Missing usage or prices remain unknown. |

## Start a hosted conversation

1. Open **Chat** and select a project.
2. Choose an available model. Connect your account in **Connections** if required.
3. Submit a message and follow the streamed response.
4. Answer prompts or exact effect approvals when requested; use the run's cancel
   control to stop work.

Execution loads when work needs it. Reading saved history or choosing a model does
not keep a runtime running. Accepted work continues when you close the browser.

Attach up to four UTF-8 text, Markdown, CSV, or JSON files per submission, each no
larger than 256 KiB. Attachments use protected account-owned storage; they do not
create a filesystem or shell environment.

In **Hosted**, **Load runtime** requires Execute access. **Release idle runtime**
also requires Control and refuses active or uncertain work. Release preserves your
conversations and managed files; new work loads execution again.

Personal conversations and execution belong to your account in that project.
Other users, including global administrators, cannot read or control them.

## Keep hosted chats close

Open a chat's actions menu and choose **Pin** to keep it in **Pinned** above
recent history. Pinning does not open the chat or interrupt running work. Choose
**Unpin** from the same menu to return it to its date group.

Click the **Pinned** heading to collapse or expand its list. Your account's choice
is remembered for this project in the current browser. Search temporarily shows
matching pins; clearing search restores your saved collapse preference.

Pins stay visible when you page through older chats, and title search filters them
along with recent history. Archived chats leave the open list; restoring a pinned
chat returns it to **Pinned**. You can also unpin it from the archived list.

Each account can pin up to 32 chats per project. Pins are saved in this browser,
independently of other accounts and projects. Clearing browser storage removes
them; if storage is unavailable, they last only for the current browser session.

## Work with a connected agent

1. Open **Fleet**, choose a host, then a workspace.
2. Open a thread or start a conversation on that agent.
3. Submit a message and follow its task/run. Wait for the first turn to establish
   its session before sending a follow-up.

A conversation stays assigned to its original agent and workspace. Saved history
remains readable while the agent is offline; later messages queue for that same
agent. Disconnection does not move accepted work to another machine.

Rename and archive require Control access. Archive hides the conversation without
cancelling its work. History may show an explicit output limit or incomplete
excerpt; continuation still uses the runtime's canonical session context.

Local Desktop and CLI history is private until its owner explicitly shares it.
See [conversation sharing](connect-runtimes.md#share-local-conversations).

## Share a page or return later

Copy the browser address for a project, agent, conversation, or task. The recipient
must sign in with their own existing permissions. Direct links survive sign-in;
browser Back and Forward follow your navigation history.

If a run reports an unknown outcome, inspect the original run before submitting it
again. A lost connection can leave an action completed remotely without a receipt.

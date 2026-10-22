---
title: Projects and access
description: Manage Cloud accounts, project membership, deployment markings, announcements, and policy monitoring.
audience: operator
type: how-to
icon: lucide/users
---

# Projects and access

Use **Administration** to manage users and projects. A global administrator can
manage the service and shared project resources; personal hosted conversations and
execution remain private to their owners.

## Create a project and grant access

1. Open **Administration → Projects** and choose **New project**.
2. Create the project, then open its **Access** view.
3. Add each member with the role needed for their work.

| Role | Project permissions |
| --- | --- |
| Viewer | Read |
| Operator | Read, execute, and control |
| Approver | Read, control, and answer effect approvals |
| Project administrator | Read, execute, control, and administer the project |

Membership is explicit for every project, including child projects. Parent access
does not grant child access. Archived projects retain history and administration
while execution and control are disabled.

Project permissions intersect the connected runtime's local grant. For example,
Approver access cannot authorize a tool the runtime has not exposed. A personal
hosted owner with Execute and Control can answer their own exact one-use approval;
shared execution retains the separate project Approve requirement.

## Manage sign-in

Local accounts are the Helm default. Bootstrap creates the first administrator
only; changing its bootstrap Secret does not reset an existing password. Manage
subsequent accounts and credentials in Administration.

For GitLab or Keycloak sign-in, follow [OIDC configuration](deployment/configuration.md#add-oidc-sign-in).
Accounts bind to verified issuer and subject, never automatically to an email address.
Changing the issuer or client ID requires users to sign in again.

## Configure hosted connections

Project administrators use **Chat → Settings** to configure providers, model
profiles, and remote MCP connections within the deployment's allowed origins.
Users manage personal accounts in **Connections**. Settings expose connection
metadata, not credentials or host file paths.

Deployment owners configure the immutable network ceiling and external keys in
[hosted chat deployment](deployment/hosted-chat.md).

## Show deployment markings

Open **Administration → Authentication & settings** to set a plain-text classification banner,
tone, and top or top-and-bottom placement. The banner appears on sign-in and
authenticated pages. It is a visual marking and does not enforce classification.
Saved settings take precedence over the server's initial `classification` seed.

To publish custom marking colors, open **Settings → Global → Appearance** as a
global administrator. Under **Classification banner colors**, choose the background
and text colors, review the preview, then select **Publish banner colors**. Both
colors must use opaque `#RRGGBB` values with at least 4.5:1 text contrast. The saved
pair applies to everyone's authenticated and sign-in banners. **Use tone colors**
restores the selected tone's theme colors. These controls leave the marking's text,
visibility and placement unchanged; personal theme and text-size preferences remain
local to each browser.

## Publish instance announcements

Global administrators publish product updates and operational notices through
**Administration → Announcements**. Every signed-in account can read published
announcements; drafts and archives remain administrator-only. These notices are
separate from the deployment classification marking.

1. Select **New announcement** and enter a title, compact banner summary and
   Markdown message. Use **Preview** to review the banner and full message before
   publication. Markdown supports links, lists and code; bodies are limited to
   16 KiB of UTF-8 text and use the shared safe renderer.
2. Choose a style and priority. Enable **Show a dismissible banner** to make the
   update eligible for the compact banner below the application toolbar. Priority
   chooses among eligible banners. The full message also appears in the toolbar's
   **Announcements** center.
3. Optionally set **Expires at** in your local time. Expired announcements leave
   the active feed and banner. Leave it empty to retain the publication until you
   archive it.
4. Select **Save draft** to retain unpublished work or **Publish to everyone** to
   release it. Edit a publication with **Publish changes**; use **Archive
   announcement** to withdraw it while retaining its administrative history.

Opening an update marks it read. Dismissing its banner also marks it read, while the
message remains available in the Announcements center. Read and dismissal state are
stored on the server for that account, so another browser or server restart preserves
them; one user's dismissal does not hide an update for another user.
Open clients refresh announcements and marking colors on focus and every 30 seconds
while visible. Use the application toolbar's refresh action to check announcements
immediately.

Publishing changed title, summary, Markdown or style creates a new content publication
and makes it unread and eligible for dismissal again. Changing priority, banner
eligibility or expiration preserves existing read and dismissal state. Archiving and
republishing identical content also preserves that state. If a save cannot be
confirmed, use **Check save status** before retrying the retained exact save; reload
the current version when another administrator has changed it.

## Monitor runtime policy

Open an agent's **Policy** view for its reported access profile, sandbox, approval
mode, grant, and configured models. Older or disconnected observations are marked
stale. These reports are not independent attestation.

Project administrators can set a monitoring baseline for sandbox profile, approval
modes, and tool ceiling. Drift is displayed without changing local enforcement.
Failed runs describe execution outcomes, not policy violations.

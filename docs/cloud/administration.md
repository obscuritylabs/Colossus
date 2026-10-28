---
title: Projects and access
description: Manage Cloud accounts, project membership, deployment markings, announcements, and policy monitoring.
audience: operator
type: how-to
icon: lucide/users
---

# Projects and access

Use **Control Plane Admin** for instance users, projects, and shared resources.
Use **Project Admin** for the selected project's settings. Personal hosted conversations
and execution remain private to their owners, including from global administrators.

## Create a project and grant access

1. Open **Control Plane Admin → Projects** and choose **New project**.
2. Create the project, then open its **Access** view.
3. Add each member with the role needed for their work.

| Role | Project permissions |
| --- | --- |
| Viewer | Read |
| Operator | Read, execute, and control |
| Terminal operator | Read, execute, control, and open human shells on enabled hosts |
| Approver | Read, control, and answer effect approvals |
| Project administrator | Read, execute, control, and administer the project |

Terminal access requires the Terminal operator role and
[local host enablement](connect-runtimes.md#enable-a-web-shell). Operator, Approver,
Project administrator, and global administrator access do not imply Terminal permission.

Membership is explicit for every project, including child projects. Parent access
does not grant child access. Archived projects retain history and administration
while execution and control are disabled.

Project permissions intersect the connected runtime's local grant. For example,
Approver access cannot authorize a tool the runtime has not exposed. A personal
hosted owner with Execute and Control can answer their own exact one-use approval;
shared execution retains the separate project Approve requirement.

## Navigate administration

Each administration entry point opens a section list with the options your account can
access. Collapse either sidebar independently. On narrow screens, use **Toggle
administration navigation** to open the section list.

Create and edit forms have separate addresses. Copy the browser address to share a
provider, model, MCP, Search, variable, secret, user, or announcement form with another
authorized person. A link or refresh loads the saved resource in its exact scope.
Addresses never contain drafts or secret values.

When leaving a form with unsaved changes, choose whether to discard them. Failed saves
keep the proposal. After a conflict, review the current settings before writing again.

## Review a user

Open **Control Plane Admin → Users** and select a user's name. Their page shows account
status, administrator access, contact details, login identities, and account timestamps.
Choose **Edit user** to change the account or reset a local password.

Use **Projects** to review explicit memberships, roles, and permissions, or follow
**Manage access** to the project's access view. Global administrator access is
separate from these memberships; it does not manufacture an execution role.

The **Workspaces** and **Chats** views have a project selector and searchable,
sortable tables with column visibility and row pagination. Search and sorting apply
to loaded records; choose **Load more** when another server page is available.
Each view has its own address, so bookmarks and browser history retain the selected
user, view, and explicit project selection.

Personal resource inventories remain owner-only, including chat titles. Reviewing
another account shows a privacy notice instead of a resource count or private rows.
Your own profile can list your authorized personal resources; opening a chat still
requires the ordinary project and owner checks.

## Manage sign-in

Local accounts are the Helm default. Bootstrap creates the first administrator
only; changing its bootstrap Secret does not reset an existing password. Manage
subsequent accounts and credentials in Administration.

For GitLab or Keycloak sign-in, follow [OIDC configuration](deployment/configuration.md#add-oidc-sign-in).
Accounts bind to verified issuer and subject, never automatically to an email address.
Changing the issuer or client ID requires users to sign in again.

## Configure hosted connections

Global administrators configure **Providers**, **Models**, **MCP servers**, **Search**,
and **Model routing** under **Control Plane Admin**. Projects inherit these definitions.
Under **Project Admin**, project administrators can customize an inherited
definition, add a project definition, or remove one from the project. Expand the
inherited configuration list to reset an override or enable a removed global definition.
Saving requires the revision that was reviewed; reload if another administrator has
changed it. Settings updates wait for hosted work to be idle.

Deployment-seeded definitions and model routes are explicit project overrides,
including the primary model. To adopt a published instance route, reset that
project override in the inherited configuration list. Review **Model routing**
before inviting users: its primary route should point to the intended inherited
or project model. Users then supply their private credentials, or use a shared
credential already assigned by an administrator.

### Assign credentials

Choose the connection's authentication mode:

- **Personal:** each user supplies a private key in **Settings → Personal → Credentials**
  or connects an account through **Connected accounts**.
- **Shared:** an administrator assigns a key to the connection. Project keys are managed
  in **Project Admin → Secrets**.
- **None:** the service accepts anonymous requests.

For an instance key, open **Control Plane Admin → Secrets**, save the key, choose a
project, and assign it to an eligible shared connection. An instance assignment takes
precedence over project keys and cannot be replaced by a project administrator.
Rotating or removing that key affects every assignment, even if the page shows only
one project's connections. Saved secrets are encrypted and never returned to the browser.
Creating a connection definition alone does not grant access to a credential.

### Set variables and Search routes

**Variables** stores nonsecret configuration values at instance, project, or personal
scope. Projects inherit instance values and can override or reset them. Personal values
are private to one account in the selected project. Variable names contain up to 128
letters, digits, or underscores; values are limited to 4,096 UTF-8 bytes. They are stored
configuration and are not automatically injected into runtime environments.

Under **Search**, configure SearXNG or SerpAPI and select the agent and research routes.
Search uses the same explicit personal or shared credential assignments as other
connections.

### Control project network access

Open **Project Admin → Network access** to review the project's connection policy.
**Allow all configured connections** is the default for existing and new projects.
This permits the origins required by configured providers, MCP servers, and Search
connections. It does not expose additional tools, grant arbitrary network effects,
or supply credentials.

To restrict a project, choose **Configure network access**, select **Restrict to an
origin list**, and enter canonical origins, one per line, such as `https://api.example.com`
or `https://search.example.com:8443`. Include any separate OAuth metadata and token
server origins required by your MCP connections. Do not include paths, queries,
fragments, or wildcards. An empty list blocks all network connections. After saving,
review unavailable connections and model roles on the Network access page. The change
uses the same revision and idle-work checks as other project settings.

Blocked definitions remain visible for review and editing. Models and roles that
depend on a blocked provider are unavailable for execution; an explicitly blocked
role does not silently use the primary model. Changing the policy does not copy or
grant access to a credential.

Control Plane catalog publication is independent of these project policies. A global
administrator can publish a connection even when a project blocks its origin. Each
project applies its own policy when resolving executable connections.

OAuth requests still require known exact origins. With the default connection policy,
an MCP server can authenticate on its own origin or through additional authentication
origins supplied by the deployment operator. For a separate identity service, configure
the service and its authentication origins in the project's explicit allow list or ask
the operator to provision them. Discovering an unknown identity-service URL does not
automatically authorize it.

Deployment owners still provision the runtime, protected storage, external keys, and
other execution authority in [hosted chat deployment](deployment/hosted-chat.md).

A newly created project can review inherited definitions before Hosted execution is
configured. Those settings remain read-only until a deployment operator supplies that
project's runtime. Creating a child project does not copy its
parent's execution authority or credentials. Secrets, personal variables, and connected
accounts show the same setup requirement; they cannot be edited or connected through
an unconfigured project.

## Configure instance telemetry

Use **Control Plane Admin → Telemetry** to configure one OpenTelemetry collector for
the instance. Projects inherit this destination; execution spans and lifecycle logs
carry project identity, while metrics remain instance aggregates. Cloud telemetry does
not export full journal payloads or private conversation content.

Save the collector configuration, then restart the Control Plane to apply it. The page
shows **restart required** while the desired configuration differs from the running
exporter. **Test collector** emits and flushes a diagnostic through the applied OTLP
configuration. A successful test confirms that diagnostic flush; it does not guarantee
future delivery. Keep telemetry disabled until the destination and transport have been
reviewed for the deployment.

## Show deployment markings

Open **Control Plane Admin → Authentication & settings** to set a plain-text classification banner,
tone, and top or top-and-bottom placement. The banner appears on sign-in and
authenticated pages. It is a visual marking and does not enforce classification.
Saved settings take precedence over the server's initial `classification` seed.

To publish custom marking colors, open **Control Plane Admin → Appearance** as a
global administrator. Under **Classification banner colors**, choose the background
and text colors, review the preview, then select **Publish banner colors**. Both
colors must use opaque `#RRGGBB` values with at least 4.5:1 text contrast. The saved
pair applies to everyone's authenticated and sign-in banners. **Use tone colors**
restores the selected tone's theme colors. These controls leave the marking's text,
visibility and placement unchanged; personal theme and text-size preferences remain
local to each browser.

## Publish instance announcements

Global administrators publish product updates and operational notices through
**Control Plane Admin → Announcements**. Every signed-in account can read published
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

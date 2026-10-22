---
title: Projects and access
description: Manage Cloud accounts, project membership, deployment markings, and policy monitoring.
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

Open **Administration → Settings** to set a plain-text classification banner,
tone, and top or top-and-bottom placement. The banner appears on sign-in and
authenticated pages. It is a visual marking and does not enforce classification.
Saved settings take precedence over the server's initial `classification` seed.

## Monitor runtime policy

Open an agent's **Policy** view for its reported access profile, sandbox, approval
mode, grant, and configured models. Older or disconnected observations are marked
stale. These reports are not independent attestation.

Project administrators can set a monitoring baseline for sandbox profile, approval
modes, and tool ceiling. Drift is displayed without changing local enforcement.
Failed runs describe execution outcomes, not policy violations.

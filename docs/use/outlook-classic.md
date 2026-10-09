---
title: Connect classic Outlook
description: Add the signed Outlook Classic plugin and connect its Windows user-session companion.
audience: user
type: how-to
---

# Connect classic Outlook

Use Windows Desktop with classic Outlook and a published Outlook Classic plugin version
that includes the session companion (alpha.4 or later).

1. Open **Plugins → Add plugin → OCI registry**.
2. Paste the platform-specific reference from the
   [plugin releases](https://github.com/obscuritylabs/colossus-plugins/releases).
3. Continue to verify the signature and activate the exact package.
4. Open the plugin details and choose **Connect Outlook session**.
5. Choose **Discover tools**, then request an Outlook operation.

The per-workspace connection starts the verified executable as the logged-in
Windows user and passes a fresh bearer token to Managed Local through its protected
bootstrap. The runtime connects to one loopback MCP endpoint; the ordinary
`windows_job` boundary remains in place for agent and plugin subprocesses. The
connection exposes the package's 14 current tool names and keeps ordinary MCP policy,
approval, and audit. **Disconnect Outlook session** stops the helper and rotates the
token on the next connection. Disabling or removing the active plugin through Desktop
revokes running Outlook helpers; downloading an update leaves the current session in
place until a different digest is activated. Restart the workspace to use a newly activated digest. A CLI activation change revokes a running helper within 15 seconds.
The earlier alpha.3 package does not contain this companion.

**Discover tools** checks the authenticated MCP transport and allowlist. Outlook COM
attachment is checked when an authorized Outlook tool runs; discovery alone does not
prove that classic Outlook is open.

See [Agent Plugins](../extend/plugins.md) for workspace selection and
[Distribute and trust plugins](../extend/plugin-distribution.md) for registry configuration.

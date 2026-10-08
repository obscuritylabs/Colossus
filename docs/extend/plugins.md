---
title: Agent Plugins
description: Load a workspace plugin, add a signed package, and choose its skills or connections.
audience: user
type: how-to
icon: lucide/puzzle
---

# Agent Plugins

Plugins give Colossus skills and optional MCP connections. Use a local directory for
project instructions, or add a signed OCI package for reuse across workspaces.

## Load a workspace plugin

Put a plugin under `.agents/plugins/` in your project:

```text
.agents/
└── plugins/
    └── review/
        ├── plugin.json
        └── skills/
            └── review/
                └── SKILL.md
```

Create `.agents/plugins/review/plugin.json`:

```json
{
  "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
  "name": "review-tools",
  "description": "Code review instructions for this project"
}
```

Create `.agents/plugins/review/skills/review/SKILL.md`:

```markdown
---
name: review
description: Review changes for correctness and missing tests.
---
Read the changed code and its callers. Explain concrete problems with file references.
Check whether tests cover the behavior that changed.
```

Colossus discovers the directory automatically. Accept the source once:

```bash
colossus plugins add .agents/plugins/review
```

Review the source approval prompt. Acceptance applies to this workspace and later
instruction edits in this directory. Local sources are shown as **Workspace source**
and remain unsigned. Acceptance grants access to their instructions; tools and
connections still need their own permissions.

In Desktop, open **Plugins**, select the discovered source, and choose
**Use workspace source**. To register another directory inside the project, choose
**Add plugin → Plugin directory**.

Now select the skill in the composer:

```text
@review-tools/review Review the changes on this branch.
```

For a conversation selection, use `/plugin use review-tools/review` in the terminal
or **Use in this conversation** in Desktop.

## Discover local sources

| Location | Behavior |
| --- | --- |
| `.agents/plugins/NAME/plugin.json` | Discovers each immediate plugin directory |
| `.agents/plugin.json` | Discovers one plugin rooted at `.agents` |
| Another directory inside the workspace | Register it with `colossus plugins add PATH` |

Choose either the single-plugin layout or the collection layout. Discovery does not
search arbitrary subdirectories. Each plugin needs its own `plugin.json`.

Browse sources with `colossus plugins list` or `/plugins`. A discovered source stays
unavailable until accepted. Missing or malformed sources show diagnostics; valid
siblings remain discoverable. SDK hosts need an explicit Colossus home to persist
acceptance, and do not implicitly open your personal home.

## Add a signed package

Use a published OCI reference:

```bash
colossus plugins add oci://ghcr.io/obscuritylabs/colossus-plugin-outlook-classic:VERSION
```

Replace `VERSION` with the published tag for your platform. In Desktop, choose
**Add plugin → OCI registry**, paste the reference, and continue.
See [Connect classic Outlook](../use/outlook-classic.md) for Windows session setup.

Colossus verifies the signature, installs the package, and activates that exact version
in one flow. The built-in GHCR profile accepts the Obscurity Labs plugin signing
workflow. For another registry, [configure a registry profile](plugin-distribution.md#registries-and-trust);
use `--registry NAME` when several profiles match.

Installed packages are shared by workspaces using the same Colossus home. Accepting
a local source with the same plugin name selects it only in this workspace. The global
version remains active elsewhere. The bundled `colossus` name is reserved.

## Control availability

```bash
colossus plugins workspace-disable .agents/plugins/review
colossus plugins disable example-plugin
```

The first command disables a local source in this workspace. The second disables an
installed plugin across the shared home. Reaccept a local source with `plugins add PATH`;
reactivate an installed version from its Desktop details or with
`plugins enable NAME --digest sha256:MANIFEST_DIGEST`, using the digest reported by
`plugins show NAME`.

Workspace [plugin settings](../reference/configuration/extensions.md) can disable
discovery, exclude names, or allow only selected names. In Desktop, open
**Settings → Workspace → Plugins** and apply the workspace changes.

## Edit and reload

Instruction and resource edits in an accepted directory apply to the next run.
Each running task keeps an immutable snapshot, including the exact instructions
and resources it started with. Disabling or removing a source affects subsequent runs.

Replacing the source directory or changing its manifest name requires fresh acceptance.
If a selected local source becomes invalid, Colossus reports it as unavailable.
Disable that local selection explicitly to use the globally active version again.

## Connect tools

A plugin can declare MCP servers, but adding it does not start them or expose their tools.
In Desktop, open its details and choose **Enable all plugin tools**, or
**Configure plugin connections** to choose an exact tool list and credentials.
Apply the settings, restart the workspace, then use **Test connection**.

For local plugins, connection permission is tied to the exact snapshot. After an edit,
review and enable the connection again. Credentials and tool permissions from an installed
version are not reused for a local source.

See [MCP configuration](../reference/configuration/extensions.md) for
`plugins.mcpServers` and the `workspacePluginDigest` binding.

## Next steps

- [Choose and inspect skills](../use/skills-plugins.md).
- [Create a plugin](plugin-authoring.md), including resources and icons.
- [Distribute and trust plugins](plugin-distribution.md), including private registries,
  offline import, signature profiles, updates, and export.
- [Agent Plugin formats](../reference/extension-formats.md) for exact contracts.

Desktop's **Developer tools → Install** and `plugins install` retain the advanced
install-only workflow: candidates stay disabled until explicitly activated.

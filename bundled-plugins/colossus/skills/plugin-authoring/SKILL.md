---
name: plugin-authoring
description: Create or update portable Agent Plugins with Agent Skills or MCP servers, validate them with Colossus, and package them as OCI artifacts for registry or offline distribution.
---
# Author a Colossus Agent Plugin

Work in the user's selected workspace. Inspect any existing plugin before changing it,
and preserve its identity and unrelated resources. The installed core plugin is read-only;
copy a relevant template into a new workspace directory before editing.

1. Establish the plugin's purpose and whether it needs skills, MCP servers, or both.
   Prefer a skill for instructions and reusable resources; use MCP when a tool connection
   is needed. Read [portable-format.md](references/portable-format.md) for the fixed
   layout and component rules. Choose the smallest relevant template under
   `assets/templates/`: `skills-only`, `stdio-mcp`, or `http-mcp`.
2. Write a root `plugin.json` and focused skill frontmatter. Keep instructions in
   `skills/<name>/SKILL.md`, optional component configuration in root `mcp.json`, and
   detailed material in the skill's resources. Rename template identifiers together.
3. Read [colossus-runtime.md](references/colossus-runtime.md) when adding scripts,
   credentials, or MCP configuration. Portable content declares no Colossus authority.
   Use the user's existing permitted filesystem and process tools to author and test.
4. Run `colossus plugins validate <directory>` and inspect component diagnostics as well
   as command success. Fix unintended diagnostics, then exercise any actual scripts or
   MCP server independently using the configured execution boundary.
5. Read [oci-distribution.md](references/oci-distribution.md), then run
   `colossus plugins package <directory> --output <new-layout-directory>`.
   Report the canonical manifest digest and validation outcome. Signing, installing,
   enabling, and pushing are separate actions performed when requested.

Use local bundled references while offline. These describe Agent Plugins v1 and the
Colossus whole-plugin OCI profile; the linked Agent Skills OCI draft is background,
not a claim that individual-skill artifacts can be installed by Colossus.

Return the authored location, skill IDs, relevant MCP configuration requirements,
validation evidence, OCI digest when packaged, and any untested dependencies.

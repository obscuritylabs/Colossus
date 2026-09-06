# Portable format

Author against Agent Plugins 1.0.0:
https://agent-plugins.org/specification
Agent Skills frontmatter: https://agentskills.io/specification

The root `plugin.json` requires `$schema` equal to
`https://agent-plugins.org/schemas/1.0.0/plugin.schema.json` and a plugin `name`.
Names are 1–64 lowercase letters, digits, hyphens, or periods, start/end alphanumeric,
and contain no consecutive hyphens or consecutive periods. `colossus` is reserved
for the bundled core. Version, description, author, license, repository, homepage,
keywords, and namespaced extensions are optional portable metadata.

Skills are discovered only at `skills/<skill-name>/SKILL.md`. Frontmatter needs
`name` matching the directory and a nonempty `description` explaining when to use
the skill. Skill names use lowercase letters, digits, and single hyphens, up to 64
characters. Keep descriptions under the 1024-character limit. Optional fields are
`license`, `compatibility`, string-valued `metadata`, and advisory `allowed-tools`.
Use uppercase `SKILL.md`. Keep detailed references and reusable templates beside it.

MCP configuration belongs only at root `mcp.json`, with `$schema` equal to
`https://agent-plugins.org/schemas/1.0.0/mcp.schema.json` and an `mcpServers` map.
Use `type: stdio` with a command and arguments, or `type: http` with a URL.
Package-relative command/cwd paths begin with `./` and stay inside the plugin.
Legacy SSE declarations produce an unsupported-component diagnostic in Colossus.

Do not add `skills`, `mcpServers`, triggers, permissions, executable tools, dependencies,
or custom Colossus fields to `plugin.json`. Client extensions belong under an extension
namespace. Colossus does not use `.codex-plugin/plugin.json` as its portable manifest.

An invalid root manifest rejects the plugin. Individual invalid skills or MCP servers
produce diagnostics without disabling unrelated valid components. For authoring,
resolve every unintended diagnostic before handing off the package.

## Optional Colossus icon

Agent Plugins v1 does not define a portable icon field. To display an icon in Colossus,
use `extensions["com.obscuritylabs.colossus"].icon` in `plugin.json`, with the value
`com.obscuritylabs.colossus/icon.png`, and include that file in the package. Use a square
PNG (128 × 128 recommended), at most 64 KiB and 512 × 512 pixels. The normalized image
must also fit within 64 KiB. URLs, absolute paths, traversal, links and SVG are rejected.
Clients that do not implement this namespace ignore it. Missing or invalid icons fall
back to a monogram in Colossus; validation reports invalid icons without disabling valid
skills. Icons are bundled display assets and grant no runtime authority.

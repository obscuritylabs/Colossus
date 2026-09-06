# Colossus runtime integration

Skill IDs are `<plugin-name>/<skill-name>`. A leading `@example-plugin/review` selects
one skill for a message; `/plugin use example-plugin/review` selects it for the current
TUI conversation. Installation alone does not activate third-party plugins.

The model sees bounded skill descriptions before selection. Selected instructions and
requested resources are loaded progressively. Resources are read by contained relative
paths; binary resources remain addressable files. Do not inline large data or binaries
into instructions. `allowed-tools` never enables a tool or bypasses runtime policy.

Scripts use ordinary shell/process tools with an explicitly selected interpreter when
needed; Colossus does not infer one or run plugin-specific executors. Plugin files remain
immutable. Runtime read/execute grants for selected plugin roots are still narrowed by
the normal tool, approval, timeout, output, and audit rules.

Plugin MCP IDs are `<plugin-name>/<server-name>`. A separate runtime configuration
entry under `plugins.mcpServers` must enable each server and allow its tools.
Portable configuration contains no credentials. Put credential references, header/env
overlays, OAuth settings, and limits in Colossus configuration, never in the package.

`${PLUGIN_ROOT}` identifies immutable installed content; `${PLUGIN_DATA}` identifies
the plugin's stable writable data directory. Expansion is single-pass; reserved
environment variables cannot be overridden by manifest or credential overlays.
Data survives updates and ordinary uninstall. A request to purge data is explicit.

Bundled trust derives from the installed Colossus executable. Other plugins use configured
Sigstore trust policies and digest verification; a matching name is never evidence of trust.

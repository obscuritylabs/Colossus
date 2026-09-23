---
name: help
description: Explain how to use and configure Colossus, or troubleshoot Colossus installation, provider, permission, plugin, worker, and state issues using documentation bundled with the installed release.
---
# Colossus help

Use the local [documentation index](references/docs/index.md) and read only the pages
relevant to the user's question. `references/docs/` is the complete repository `docs/`
tree embedded when this Colossus executable was built, including diagrams and assets.
Its directory structure and relative links are preserved. Prefer this snapshot for
the installed release; the public documentation site may describe a newer version.

Choose an entry point:

- Setup and everyday use: [Get started](references/docs/get-started/index.md),
  [Desktop](references/docs/get-started/desktop.md), and
  [terminal UI](references/docs/use/terminal-ui.md).
- Configuration: [configuration reference](references/docs/reference/configuration.md).
- Failures and recovery: [troubleshooting](references/docs/admin/troubleshooting.md).
- Extensions: [plugins](references/docs/extend/plugins.md) and
  [MCP](references/docs/extend/mcp.md).
- Implementation questions: [developer guide](references/docs/develop/index.md).

For troubleshooting, establish the affected interface, installed version, selected
workspace/configuration, and exact symptom before choosing the relevant diagnostic.
Use the troubleshooting guide's symptom map and the affected feature's reference.
Inspect reported readiness and individual checks, not just command exit status.
Distinguish local inspection from diagnostics that contact a provider or send a model
request. Apply corrections within the user's requested scope and existing permissions,
then verify the original symptom. Follow the documented recovery procedure for state
or audit failures. If unresolved, prepare the guide's bounded, sanitized issue report.

Use skill resource listing and reading to navigate the snapshot. Large text files and
binary assets may exceed the resource preview limit; inspect relevant portions with
already-permitted filesystem tools under this selected skill's immutable root when
available. Cite the bundled page and section used, and state any missing evidence.
If diagnosing a different Colossus version or a remote worker, establish that mismatch
and use documentation for the target version before recommending version-specific changes.

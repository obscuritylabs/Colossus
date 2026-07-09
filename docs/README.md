# Colossus Documentation

Colossus is a secure, local-first CLI harness for agentic development. Start here if
you are trying to install it, use it day to day, configure credentials, or understand
how the codebase is put together.

## Start Here

- [Getting Started](GETTING_STARTED.md): install from a source checkout, run the echo
  smoke test, start the REPL, choose a workspace, and understand approval modes.
- [User Guide](USER_GUIDE.md): everyday CLI and REPL usage for sessions, tools, context,
  memories, skills, research, and integrations.
- [Workflows](WORKFLOWS.md): copyable recipes for common Colossus jobs.
- [Troubleshooting](TROUBLESHOOTING.md): local model, auth, tool, context, and network
  failure patterns.

## Capabilities

- [Built-in Tools](TOOLS.md): model-callable tool families, permissions, schemas, and
  security notes.
- [Integrations](INTEGRATIONS.md): GitHub, OpenAPI imports, MCP positioning,
  credential refs, and auth boundaries.
- [Skills](SKILLS.md): skill authoring, resources, required tools, and Skill Mode.
- [Packs](PACKS.md): installable capability packages and executable boundaries.
- [Context Compaction](CONTEXT.md): session history, snapshots, summaries, and context
  model behavior.

## Operators

- [Configuration](CONFIGURATION.md): full config schema, provider/model roles,
  workspace, HTTP, credentials, approvals, and compaction settings.
- [Security Model](SECURITY.md): trust boundaries, tool execution rules, approvals,
  integration credentials, audit logs, and bundle handling.
- [Offline and Airgapped Operation](OFFLINE_AIRGAP.md): offline-safe workflows and local
  model endpoint setup.
- [Offline Bundle Format](BUNDLE_FORMAT.md): bundle layout, manifest schema, and
  verification.
- [Release Process](RELEASE.md): release readiness, artifact review, tags, and
  post-release checks.

## Developers

- [Architecture](ARCHITECTURE.md): ports-and-adapters layering and service boundaries.
- [Product Requirements And Reconstruction Specification](FEATURE_INVENTORY.md):
  implementation-neutral product contract, complete feature baseline, milestones, and
  acceptance checklist for a clean reconstruction.
- [Contributing](CONTRIBUTING.md): commit message expectations.
- [Installation](INSTALLATION.md): source checkout and platform paths.

## Documentation Principles

- Put user journeys in the user docs first, then link to reference pages.
- Keep the root README short: product overview, quick start, and docs links.
- Keep secrets and credential values out of examples. Use refs such as
  `env:GITHUB_TOKEN`.
- When a CLI or REPL command changes, update the user guide, relevant feature page, and
  configuration or security reference if the behavior affects those boundaries.

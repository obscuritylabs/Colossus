---
title: Agent Plugins
description: Validate, distribute, trust, install, and run Agent Plugins over OCI.
audience: operator
type: guide
---

# Agent Plugins

Colossus implements [Agent Plugins v1](https://agent-plugins.org/specification) as its
only portable extension package. A plugin is one directory with required `plugin.json`,
optional Agent Skills at `skills/NAME/SKILL.md`, optional root `mcp.json`, and arbitrary
resources. Skills use the [Agent Skills specification](https://agentskills.io/specification).

Plugins are installed once beneath `$COLOSSUS_HOME/plugins` and shared by every workspace
using that home. Ordinary installation leaves a plugin disabled. Enabling selects one immutable OCI
manifest digest globally; each workspace can narrow that active set with `plugins.enabled`,
`plugins.include`, and `plugins.exclude`.

## Built-in skills

The CLI and Managed Local sidecar embed the `colossus` plugin, including `coding`,
`offline-dev`, `security-review`, and `plugin-authoring`. First startup with an explicit
Colossus home installs and enables it without a checkout, registry or interpreter.
Standalone SDK runtimes without a home remain isolated and do not open your personal home.

Core uses the same immutable, digest-addressed store as imported plugins. Its inventory
label is **Bundled with Colossus**, not a claim of Cosign signature verification. Only
compiled content can receive that ownership; importing a directory named `colossus`
does not. Core can be inspected, verified, exported, enabled or disabled, but its version
is managed by the executable, not independent update or uninstall commands.

On startup, each binary selects its bundled version for subsequent runs sharing that
home, including binary rollbacks. A user's global disabled preference survives that
change. Workspace exclusions never change global activation. Existing runs retain their
original leased catalog, skill instructions and MCP configuration.

## Desktop and terminal selection

Desktop's **Plugins** surface lists installed candidates, availability, source, digest,
trust, component diagnostics, skills and MCP servers. Managed Local owns lifecycle
operations and native import/export dialogs. External targets provide authorized
discovery and bounded previews when they advertise support; they are not managed locally.
Public API clients can request unavailable metadata with `include_disabled`; instruction
and resource reads still require the plugin to be available in the workspace. An empty
kind filter or `EXTENSION_KIND_UNSPECIFIED` includes Agent Plugins.

Use **Use in this conversation** for a sticky selection, or start one message with
`@colossus/plugin-authoring` for a message-only selection. Sticky and message selections
are combined without duplicates. Unknown mentions remain ordinary text. New conversations
and target changes do not inherit selections; unavailable selected skills fail explicitly.
Desktop captures recognized mentions before queuing a message. Later delivery validates
those IDs again; disabling a plugin cannot silently turn a queued skill selection into
ordinary prompt text.

In the TUI, `/plugins` opens inventory and `/plugins OPERATION` accepts the same management
arguments as the CLI. `/plugin skills`, `/plugin active`, `/plugin use PLUGIN/SKILL`,
`/plugin remove PLUGIN/SKILL`, and `/plugin clear` manage conversation selections.
`/plugin show`, `/plugin resources`, and `/plugin read` provide progressive disclosure.
Selecting a plugin name lists its skills; it does not select every skill.

Installation and activation are separate. An update installs a candidate; activate the
exact digest explicitly. An untrusted-content checkbox only requests approval and is
never approval evidence. Native policy prompts identify the operation's scope. Browsing
instructions or configuring credentials never enables MCP servers.

## Portable layout

```text
example-plugin/
├── plugin.json
├── skills/
│   └── review/
│       ├── SKILL.md
│       ├── scripts/
│       └── references/
└── mcp.json
```

Colossus discovers only those fixed locations. Skill IDs are always qualified as
`PLUGIN_NAME/SKILL_NAME`; unqualified selections are rejected. All skill metadata is
available for discovery, while the `SKILL.md` body is loaded only when selected or read
through `plugin.skill.read`. Text resources are bounded previews; binary resources remain
listed by contained path.

`allowed-tools` is advisory metadata. It cannot expose a hidden tool, bypass policy, or
grant filesystem, process, network, credential, or approval authority. Scripts run only
through the ordinary shell/process tools with the selected immutable plugin root added as
a read/execute grant.

## Validate, package, and install

```bash
colossus plugins validate ./example-plugin
colossus plugins package ./example-plugin --output ./example-plugin.oci
colossus plugins verify ./example-plugin.oci --trust-profile default
colossus plugins install --layout ./example-plugin.oci --trust-profile default
colossus plugins enable example-plugin --digest sha256:MANIFEST_DIGEST
```

Directory, OCI layout, deterministic layout tar, and registry-reference installs are
supported. Layouts with multiple candidates require `--digest`. Tags are resolved once;
the recorded identity is always the verified OCI manifest digest.

The Colossus OCI profile uses a standard OCI image manifest with:

- `artifactType: application/vnd.colossus.agent-plugin.v1`
- config `application/vnd.colossus.agent-plugin.config.v1+json`
- one `application/vnd.colossus.agent-plugin.content.v1.tar+gzip` layer
- exactly one archive root named for the plugin

Packaging is deterministic. Import rejects image indexes as plugin payloads, multiple
content layers, traversal, absolute and duplicate paths, links, devices, special files,
digest/size mismatches, oversized manifests or files, more than 10,000 files, and more
than 2 GiB extracted content.

## Registries and trust

Registry profiles declare an exact origin, allowed token-service and blob-redirect
origins, per-origin CA roots, authentication, and a trust profile. No registry is contacted
at startup and no ambient Docker credentials are used unless `auth.kind: docker` is
selected explicitly. Bearer/basic values remain credential references. Docker helpers
require an exact configured executable and run through the normal process permit and audit
boundary.
Docker configuration is opened only inside an authorized registry transfer, and its file
must be covered by that transfer's permit. Denied transfers do not inspect credentials.

Trust profiles are `required` by default. `optional` admits unmatched content as untrusted;
enabling it requires explicit approval. `disabled` deliberately applies digest integrity
only and has the same explicit untrusted-enable requirement. Signature verification is
in-process Sigstore/Cosign using configured public keys or keyless issuer/subject identity,
with optional local trust roots and bundled transparency evidence for disconnected use.
Verification and installation require read grants for the selected profile's public-key
and trust-root files. The built-in policy requests approval for paths outside existing
workspace grants; an external policy must supply those grants explicitly. Re-verifying
an installed plugin uses the same checks.

```bash
colossus plugins pull registry.example/acme/review:v1 \
  --registry production --output ./review.oci
colossus plugins push ./review.oci registry.example/acme/review:v1 \
  --registry production
colossus plugins install --reference registry.example/acme/review@sha256:DIGEST \
  --registry production
```

Cosign signatures and attestations remain standard OCI 1.1 referrers. Colossus does not
invent a signing envelope.

## MCP enablement and data

Portable MCP server IDs are `PLUGIN_NAME/SERVER_NAME`. Every server needs a matching
`plugins.mcpServers` entry with `enabled: true` and an explicit `allowedTools` list before
any tools are exposed. Colossus supports stdio and Streamable HTTP. A valid legacy SSE
entry is reported independently as unsupported and does not disable the plugin.

Managed Local's plugin details offer explicit connection testing and OAuth status/sign-in
for enabled servers. Save and apply the server overlay first. These actions reuse the
normal native credential and OAuth boundaries and never enable a plugin or MCP server.
Runtime effect names use length-prefixed plugin and server components so dotted names
cannot accidentally share permission rules; portable server IDs remain `PLUGIN/SERVER`.

`${PLUGIN_ROOT}` and `${PLUGIN_DATA}` expansion is exact, single-pass, and limited to MCP
arguments, environment values, and `cwd`. Reserved variables are set after manifest and
client overlays. Plugin roots stay immutable; stable writable state lives at
`$COLOSSUS_HOME/plugins/data/PLUGIN_NAME` and survives update or uninstall unless
`--purge-data` is explicit.

## Lifecycle and air gaps

```bash
colossus plugins list
colossus plugins show example-plugin
colossus plugins disable example-plugin
colossus plugins uninstall example-plugin --digest sha256:DIGEST
colossus plugins gc
colossus plugins export example-plugin --output ./example-plugin-layout.tar
```

The dedicated `plugins/state.redb` journal serializes lifecycle writers. Running snapshots
lease their immutable content, so disable, uninstall, or garbage collection affects only
later runs. Export carries the plugin manifest, blobs, signatures, and attestations and
does not open a network connection during import.

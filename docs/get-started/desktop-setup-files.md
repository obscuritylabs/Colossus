---
title: Desktop setup files
description: Share an offline Desktop setup with providers, models, global defaults, MCP, search, telemetry, and optional certificates.
audience: user
type: how-to
icon: lucide/file-cog
---

# Desktop setup files

## Goal

A Desktop setup file is a ZIP archive named `NAME.colossus-setup`. Build it inside
your environment and transfer the single file to each computer. Inspection and
import are local: they do not fetch schemas, icons, models, or credentials.

## Prerequisites

- Colossus Desktop installed on the receiving computer.
- Your organization's provider endpoints, supported model settings, and token instructions.
- A ZIP utility in the environment where you prepare the package.

## Steps

Choose **Import setup file** on the first Desktop setup page or under **Settings →
Global → Providers** or **Desktop**. Review provider endpoints, model limits,
capabilities, suggested roles, instructions, and certificate fingerprints before
choosing **Import setup**. The review opens in a dialog; after import,
the Desktop page shows a compact provider/model count beside appearance settings.

Import adds every provider and model to the normal global inventory without starting
a provider or changing an existing workspace's model selection. No workspace or API
key is required to import. Connections awaiting a key show **Needs API key**; they
are not treated as connections that allow anonymous access.

Continue to **Workspace** to choose a folder, then **Provider** to see all imported
connections. The package's primary provider is preselected. Select a provider to see
its endpoint, protocol, timeout, and Markdown instructions. **Add API key** is optional
at this stage; keys are entered through native controls and remain in the encrypted
credential vault. A saved key is not a successful connection test.

The **Model** step shows the chosen provider's bundled models, with the package's
primary model selected when available. Limits and capabilities are displayed without
contacting the provider. You can also discover other models or enter an ID manually.
**Save and start** uses the existing native configuration and confirmation path.
If the chosen provider needs a key, the secure prompt asks for it then and saves it
for reuse with that imported provider. Cancelling leaves setup ready to retry.

Other imported providers remain available for later selection. The selected provider
and its models are added to the workspace; other providers' credentials are not
required. Existing workspace configurations retain their accepted defaults until
their normal configuration update is reviewed and applied.
For an already configured workspace, **Settings → Global → Providers** also offers
**Use model in workspace** inside each provider’s expanded details, with explicit
replacement for conflicting profile definitions. Key enrollment and workspace
activation show progress and errors beside the action.

Use **Manage setup files** to export a saved package, review its certificates, or
remove the saved file and instructions. Removing a saved setup keeps providers,
models, credentials, and existing workspaces. Delete unwanted providers and models
from their normal inventory actions. Older saved setups are added to the inventory
once when Desktop opens them; later inventory deletions stay deleted.

### Add provider instructions and icons

In **Settings > Global > Providers**, add or edit a provider and open **Advanced
options**. Write a Markdown description or token-access instructions and use
**Preview** to check the formatting. You can choose a PNG icon and an optional dark
theme icon. Saved instructions appear in the provider's details; its custom icon
appears wherever Desktop shows that provider.

These presentation settings belong to Desktop's setup manifest. Saving them does
not change model routing or restart a workspace. **Export global setup** includes
your current instructions and embeds the PNGs for offline import. Clearing a field
removes its custom value even if the provider originally came from a setup file.

## Package layout

The archive has no enclosing directory:

```text
company.colossus-setup
├── manifest.yaml
├── config.yaml
├── assets/
│   ├── company.png
│   └── company-dark.png
└── certificates/
    └── company-ca.pem
```

Only referenced assets belong in the archive. Icons and certificates are optional.
Do not include scripts, executables, private keys, credentials, or application state.

For a ready-to-package company example and repeatable manual checks, see
[`examples/desktop-setup/`](https://github.com/obscuritylabs/Colossus/tree/main/examples/desktop-setup).
Start with the repository's `examples/desktop-setup/manifest.yaml` and
`examples/desktop-setup/config.yaml`. The adjacent
`manifest.schema.json` describes manifest fields for offline editor assistance.
Desktop's native validation is authoritative and does not download this schema.

The example contains **five providers and six models**: Company AI, Research Gateway,
Vision Gateway, Code Lab, and Local Models. The endpoints and model IDs are examples;
replace them with your actual services before sharing the package.

## Manifest

```yaml
schemaVersion: 1
id: company-ai
name: Company AI setup
version: "1"
descriptionMarkdown: |
  Configure the models available on your company network.
providers:
  company:
    displayName: Company AI
    descriptionMarkdown: |
      ### Get a token
      Open the [AI portal](https://ai.example.com/tokens), create a token,
      then choose **Add API key**.
    icon: assets/company.png
    darkIcon: assets/company-dark.png
caBundle: certificates/company-ca.pem
```

Provider IDs must match `providers.profiles` keys in `config.yaml`. Package IDs
identify re-imports; names and versions are administrator-supplied display metadata,
not publisher verification. Re-import previews the previous version and requires
explicit replacement. Credential bindings survive only when profile ID, protocol,
endpoint, and credential placeholder match. Existing workspace configurations retain
their current values until a model is explicitly selected again.

Markdown supports ordinary headings, lists, links, emphasis, and code. HTML and
remote images are disabled. Clicking a supported HTTP(S) instruction link opens the
system browser; simply importing or rendering instructions makes no request.

Icons are static PNGs, at most 64 KiB and 512 × 512 pixels each. Desktop normalizes
their pixels before displaying or storing them. The optional `darkIcon` is used in
dark mode. Without an icon, Desktop keeps its normal provider icon or fallback.

## Provider and model configuration

`config.yaml` uses the existing schema-version-3 provider/model YAML semantics.
Its supported top-level fields are `schemaVersion`, `providers`, `models`, and,
with manifest version 2, `desktop`. The runtime schema version remains 3. Version 1
packages remain supported. A version 2 package can use `providers: {profiles: {}}`
and `providers: {}` in the manifest when it contains only global settings.

Provider fields are `kind`, `baseUrl`, `credentialReference`, and `timeoutMs`.
Use `open_ai_compatible`, `open_ai_responses`, or `open_ai_codex`. Model fields
follow [Provider and model configuration](../reference/configuration/providers-models.md).
Declare actual supported limits and capabilities. Omit `models` entirely to leave
model selection for later.

`credentialReference: env:COMPANY_AI_TOKEN` is a portable credential slot. Desktop
does not read that environment variable; the user binds a native credential. Omit
the reference or set it to null only for connections that need no authentication.
Host credential IDs and literal secrets are rejected. Codex requires
`credentialReference: codex:default`, omits `baseUrl`, and uses its existing
account sign-in flow.

## Global defaults, MCP, search, and telemetry

Choose **Export global setup** after configuring your global settings. Export includes
the current, unarchived provider, model, MCP, search, and telemetry definitions and
the current global defaults. It excludes catalog history, workspace data, stored
secrets, and machine credential IDs. Archived resources are omitted. Provider and
model profile names must be unique for export; rename conflicting profiles first.
Provider instructions and icons include your saved edits, or the original details
for matching imported connections when you have not customized them.
Exporting a saved package instead preserves that package's original definitions.

The optional `desktop` section reuses Desktop's typed settings. Each catalog item
has a portable `id`, a display `label`, and a `configuration`. See the complete
[company example](https://github.com/obscuritylabs/Colossus/tree/main/examples/desktop-setup)
for MCP, search, and telemetry definitions.

```yaml
desktop:
  defaults:
    accessProfile: development
    executionBoundary: workspace_isolated
    terminalEnabled: false
    fieldOverrides:
      - fieldId: agent.maxTurns
        value: 25
      - fieldId: sandbox.timeoutMs
        value: 60000
  mcpServers: []
  searchProviders: []
  telemetryProfiles: []
```

Only Desktop-managed default fields are accepted; storage paths and other
Desktop-owned runtime invariants cannot be overridden. Imported defaults are
validated with the runtime YAML parser. **Use included global defaults** is an
explicit choice in the review. It replaces the default snapshot for new workspaces;
existing workspaces remain pinned until their normal update and authority review.
Leaving it unchecked imports the catalogs while retaining your defaults.

Advanced defaults for audit export, semantic memory, plugin registries, and plugin
MCP overlays can be shared only without authentication settings. Import and export
reject credential references in those overrides, including Docker registry
authentication, because they cannot be bound to Desktop's portable credential slots.
Configure that authentication locally, or use the MCP and search catalogs for
connections that need portable credential placeholders.

MCP, search, and telemetry entries appear on their normal global settings pages.
Import does not start MCP commands, contact search endpoints, select these entries
for existing workspaces, or enable telemetry export. Review command paths, tool
allowlists, telemetry destinations, and limits before selecting an entry for a workspace.

In portable MCP and search definitions, credential ID fields contain `env:NAME`
placeholders, including `credentialId`, `environmentCredentials`, and OAuth
`clientSecretCredentialId`. These are names for missing local credentials, not
environment variables to read. Recipients fill them through **Credentials** settings.
Exports preserve each referenced credential's original name and type in the optional
`desktop.credentials` mapping. Import creates empty local entries with those names
and types, and **Add token** fills the existing entry. Shared references within a
package share one entry; identical names alone never select an existing token.

```yaml
desktop:
  credentials:
    env:DOCS_TOKEN:
      label: Company documentation token
      kind: bearer_token
```

Each metadata key must be a slot referenced by an included MCP or search definition.
Names are limited to 96 UTF-8 bytes. Supported kinds are `api_key`, `bearer_token`,
`client_secret`, and `generic_secret`. Packages without metadata still import using
the placeholder name and `generic_secret`. Neither token values nor local credential
IDs are exported.
Literal MCP headers are converted to credential placeholders during export; their
values never enter the archive. Use `credentialHeaders` in hand-authored packages.
Do not embed tokens in command arguments, instructions, or other free text.

Reimport preserves matching local bindings when the definitions using that slot have
not changed. Changed definitions get fresh empty credential slots, so a package
cannot reuse a stored secret at a different destination. User-edited catalog entries
are preserved; replacement creates a separate entry when necessary.

## CA certificates

A package can contain one PEM bundle of public CA certificates. The review shows its
SHA-256 fingerprints and whether it would replace the current additional CA bundle.
**Trust the included CA certificates in Colossus** starts unchecked.

Trust applies to Colossus-owned connections across the app, rather than just one
provider. Import does not change the operating system trust store or subprocess
TLS configuration. Applying trust while Managed Local is configured uses its existing
restart/rollback behavior and rejects changes while managed work is active.

Leaving trust unchecked still saves the setup package. **Review setup**
lets you apply them later. Removing a saved setup does not remove trusted
certificates, credentials, or configured workspace resources.

## Build without a bundler

From the example directory, archive the files themselves. On PowerShell:

```powershell
Compress-Archive -LiteralPath manifest.yaml, config.yaml -DestinationPath company.zip
Rename-Item -LiteralPath company.zip -NewName company.colossus-setup
```

Include `assets` and `certificates` in `-LiteralPath` when your manifest references
them. On systems with the standard ZIP utility:

```sh
zip company.colossus-setup manifest.yaml config.yaml
```

Add the referenced image and PEM paths to that command. No Colossus-specific packaging
utility or network access is required.

**Export setup file** reproduces a saved package without credential bindings.
**Export global setup** captures current global definitions, defaults, and the
additional public CA bundle, with portable placeholders for stored credentials.
The recipient reviews certificate trust separately before applying it.

## Expected result

One portable file makes your providers and model details available throughout Desktop
setup. Each workspace activates only its chosen provider; other imported providers
remain available without requiring their credentials.

## Verification

Import the package into a fresh Desktop profile. Review all provider endpoints and
model details, then choose a workspace and confirm the recommended provider and model.
Confirm token instructions display correctly and CA trust starts unchecked. Add a key
when ready and use **Save and start** to configure the selected provider.

## Failure path

Packages support up to 16 providers and 64 models. Desktop retains at most four
setup packages. The compressed archive is limited to 2 MiB, expanded content to
1 MiB, and any one file to 256 KiB. Normalized icons share a 256 KiB budget.
Saved setup entries also share Desktop's existing 1 MiB settings limit.

Paths must be relative portable archive paths. Linked files, duplicate paths,
unreferenced files, unsupported versions, and unknown manifest/configuration fields
are rejected. If inspection fails, the existing configuration remains unchanged.
A model whose profile ID already has a different workspace definition needs explicit
replacement or a distinct ID in the setup file.


## Next step

Share the reviewed setup file with your users. Continue with
[Desktop setup](desktop.md) to configure workspaces and begin a conversation.

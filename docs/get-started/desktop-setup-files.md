---
title: Desktop setup files
description: Build and import an offline Desktop setup package with provider instructions, models, icons, and optional CA certificates.
audience: user
type: how-to
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
choosing **Import providers and models**. The review opens in a dialog; after import,
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
required. Existing workspace configurations are never replaced by imported defaults.
For an already configured workspace, **Settings → Global → Providers** also offers
**Use model in workspace** inside each provider’s expanded details, with explicit
replacement for conflicting profile definitions. Key enrollment and workspace
activation show progress and errors beside the action.

Use **Manage setup files** to export a saved package, review its certificates, or
remove the saved file and instructions. Removing a saved setup keeps providers,
models, credentials, and existing workspaces. Delete unwanted providers and models
from their normal inventory actions. Older saved setups are added to the inventory
once when Desktop opens them; later inventory deletions stay deleted.

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
Its supported top-level fields are `schemaVersion`, `providers`, and `models`.
Workspace permissions, tools, storage, and network grants are configured through
normal Desktop setup.

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

## CA certificates

A package can contain one PEM bundle of public CA certificates. The review shows its
SHA-256 fingerprints and whether it would replace the current additional CA bundle.
**Trust the included CA certificates in Colossus** starts unchecked.

Trust applies to Colossus-owned connections across the app, rather than just one
provider. Import does not change the operating system trust store or subprocess
TLS configuration. Applying trust while Managed Local is configured uses its existing
restart/rollback behavior and rejects changes while managed work is active.

Leaving trust unchecked still saves the setup package. **Review CA certificates**
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
**Export workspace setup** creates provider/model YAML from the selected workspace,
with portable placeholders for saved keys.

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

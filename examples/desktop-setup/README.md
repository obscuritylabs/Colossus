# Company setup example

Use this folder to try Desktop setup imports after installing a new build. It
contains five sample providers and six models, with instructions and a recommended
default, plus global limits, an MCP server, search, and telemetry. You can test import, provider selection, and model cards **without an API
key or a running model server**.

Requires a Desktop build with **setup manifest version 2** support. Desktop 0.11.2
supports version 1 provider/model packages only. The remote endpoints use fictional
`example.com` addresses, and every model ID is a placeholder. The sample is ready
for offline setup testing; customize it before testing a real conversation.

## Folder layout

```text
desktop-setup/
├── README.md                 # Packaging and manual checks
├── manifest.yaml             # Company name and provider instructions
├── config.yaml               # Connections, models, defaults, MCP, search, telemetry
├── manifest.schema.json      # Offline editor help; not part of the archive
└── output/                   # Generated locally; ignored by Git
    └── company.colossus-setup
```

The importable archive contains `manifest.yaml` and `config.yaml` at its root,
with no enclosing folder. Do not zip this entire directory: the README, schema,
and generated output are not package contents. This basic sample includes no
custom icons or CA certificates and changes no certificate trust.

| Provider | API format | Bundled model IDs | Authentication |
| --- | --- | --- | --- |
| Company AI | Chat Completions | `company/engineering`, `company/general` | API key |
| Research Gateway | Chat Completions | `research/analyst` | API key |
| Vision Gateway | Responses | `vision/reader` | API key |
| Code Lab | Chat Completions | `code/assistant` | API key |
| Local Models | Chat Completions | `example-local` | None |

Company AI / `company/engineering` is recommended. Vision Gateway demonstrates
image-input capability, and Local Models points to `http://127.0.0.1:11434/v1`.

## Build the setup file

Run either recipe from the repository root. Both create
`examples/desktop-setup/output/company.colossus-setup`. Rebuilding replaces only
that generated package; the source YAML remains editable.

### Windows PowerShell

```powershell
Push-Location examples/desktop-setup
try {
    New-Item -ItemType Directory -Path output -Force -ErrorAction Stop | Out-Null
    Compress-Archive -LiteralPath manifest.yaml, config.yaml -DestinationPath output/company.zip -Force -ErrorAction Stop
    Move-Item -LiteralPath output/company.zip -Destination output/company.colossus-setup -Force -ErrorAction Stop
} finally {
    Pop-Location
}
```

### macOS or Linux

Requires the standard `zip` utility. A fresh temporary directory ensures a rebuild
cannot retain old archive members after you change the sample.

```sh
(
  set -eu
  cd examples/desktop-setup
  mkdir -p output
  setup_archive_dir="$(mktemp -d)"
  zip -j "$setup_archive_dir/company.colossus-setup" manifest.yaml config.yaml
  mv "$setup_archive_dir/company.colossus-setup" output/company.colossus-setup
  rmdir "$setup_archive_dir"
)
```

## Manual test checklist

Use the first setup page in a fresh Desktop profile, or **Settings → Global →
Providers → Import setup file** in an already configured app. Importing alone does
not change an existing workspace's selected model, so there is no need to delete
your application data to repeat these checks.

| Action | Expected result |
| --- | --- |
| Import `output/company.colossus-setup` | A review opens for **Company AI setup**, version **2**, with five providers and six models. |
| Cancel the review | No new setup is saved and existing configuration stays unchanged. |
| Import again and choose **Import setup** | Five providers, six models, and the MCP, search, and telemetry entries appear on their global settings pages without requesting keys or contacting endpoints. Four providers show **Needs API key**; Local Models shows **No key required**. |
| Review **Global defaults**, then select **Use included global defaults** before importing | New-workspace defaults include workspace isolation, disabled terminal, 25 turns, a 60-second effect timeout, and a 1 MiB output limit. Existing workspaces keep their settings. |
| Open **Credentials** settings | The MCP and search token slots are missing until you enter them locally. |
| Choose **Export global setup**, then import it in a disposable Desktop profile | Current global catalogs and defaults appear in review. Stored credentials and workspace data do not travel with the file. |
| In the setup wizard, choose a disposable workspace folder and continue to **Provider** | All five imported providers appear; Company AI is recommended and preselected. |
| Select each provider | Its endpoint, API format, timeout, and Markdown instructions appear. Skip **Add API key** for this offline check. |
| Continue to **Model** with Company AI selected | Both company models appear; `company/engineering` is recommended and preselected. No **Load models** action is needed. |
| Go back and select Vision Gateway, then return to **Model** | The `vision/reader` model shows image support and the provider uses Responses. |
| Import the same package again | Review asks for explicit replacement of the saved setup rather than silently duplicating it. |
| Choose **Export setup file**, then inspect that exported file | The same provider/model details are present without saved API keys. Cancel if you do not want to replace the setup again. |

On an already configured workspace, expand a provider in **Global → Providers**.
Its **Use model in workspace** action tests activation separately. Use a disposable workspace for activation:
that step changes its provider/model configuration and may request a credential.

To remove the saved sample file, open **Manage setup files → Remove saved setup**
and choose **Remove Company AI setup**. This keeps providers, models, configured
workspaces, and saved credentials. Delete unwanted providers and models separately
from the inventory.

## Test a real conversation

1. Copy this example folder for your own edits. Keep the original as a repeatable
   offline test case.
2. In the copy's `config.yaml`, replace one provider's `baseUrl` and its models'
   `model` IDs, limits, and capabilities with values supported by your server.
   For Local Models, start your server and replace `example-local` with an
   installed model ID.
3. Update the matching provider instructions in `manifest.yaml`. Choose a new
   package `id` for a separate setup, or keep `company-ai` and change `version` to
   exercise replacement review.
4. Package the edited YAML using the same recipe, import it, and select that
   provider and model. Enter any API key through Desktop's **Add API key** control;
   do not put a secret in the package. The `env:` values are credential placeholders,
   not environment variables Desktop will read.
5. Complete **Save and start**, then send: `Reply with exactly SETUP_OK.`

For custom icons, public CA bundles, supported configuration fields, and package
limits, see [Desktop setup files](../../docs/get-started/desktop-setup-files.md).
For agent tasks after setup, see the [ask examples](../asks/README.md).

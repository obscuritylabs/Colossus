# Colossus for VS Code

The first VS Code MVP connects to an enrolled local Colossus worker through the
public TypeScript SDK. It provides Plan/Execute/Research chat, streamed responses, tool activity,
native approval and question dialogs, cancellation, recent conversations, and recovery
after disconnect or an extension reload. Explicit selection/file context captures the
current editor buffer, including unsaved text. **Review changes** opens VS Code's Source
Control view and its native file diffs.

Chat opens in VS Code's right secondary sidebar. **Colossus Workspace** opens in the
left activity bar with Sessions, Plans, and Runtime views. Selecting a session opens
it in chat; inspecting a run or plan opens saved state in an editor tab. Settings use
Desktop's Global/Workspace layout. Desktop and the webviews now consume the private `apps/ui` source package for design
tokens, branding, the settings frame, accessible dropdowns, and composer controls.
VS Code defaults to neutral Dark+ surfaces with Colossus accents. Light/dark theme
selection follows VS Code, with a native high contrast bridge. The composer grows automatically, uses Desktop's segmented Plan/Execute/Research
controls, and retains a draft while a run is active. Replies support bounded Markdown
including scrollable tables; model HTML, links, and images remain inert. Drafts are sent only by the user.

Models, tools, plugins, policy, sandboxing, and canonical state remain in the runtime.
This extension does not start a worker or connect to Desktop's private Managed Local
instance. Use a separately enrolled worker for the selected workspace. Desktop and
CLI/worker state remain governed by their existing interface partitions.

## Build and install

From the repository root, with Node.js 22 or newer:

```sh
cd sdk/typescript
npm ci --ignore-scripts
npm run build
cd ../../apps/vscode
npm ci --ignore-scripts
npm run package
```

The package command produces a platform-specific VSIX in `artifacts/`, including the
native OS-keyring binding. Supported build targets are x64/ARM64 Linux, macOS, and
Windows. To cross-package after building, run `node scripts/package.mjs darwin-arm64`
(or another supported target). The package script fetches the target binding from the
locked registry URL and verifies its SHA-512 integrity before staging it. On Windows,
replace shell directory commands as appropriate; packaging needs `tar` on PATH.
VS Code 1.104 or newer is required for the secondary-sidebar contribution. Install the VSIX using **Extensions: Install from
VSIX…**, then run **Developer: Reload Window** after installing or updating so VS Code
registers the contributed views and settings. If **Appearance: Palette** is missing
from VS Code's standard Colossus settings, reload the window before choosing a palette.
For source development, open `apps/vscode` in VS Code and launch the
**Colossus Extension** debug configuration after installing dependencies and building
the SDK. The extension runs in the workspace extension host; with Remote SSH or a
devcontainer, the worker and credential store must be available on that remote host.

## Enroll and connect a worker

Install and configure Colossus for the same repository that VS Code opens. See
[application connection and enrollment](../../docs/develop/application-sdk.md#connection-and-enrollment)
for the canonical authentication contract and
[worker administration](../../docs/admin/storage-worker.md#first-application-enrollment)
for configuration, rotation, and revocation.

With the worker stopped, run this from that repository. Replace the absolute discovery
directory with an owner-private directory; these names are the extension's defaults:

```sh
colossus --workspace /absolute/path/to/repository worker \
  --public-api-dir /absolute/private/colossus-vscode-api \
  --enroll-application app:colossus-vscode \
  --scope runs:execute --scope runs:read --scope runs:control \
  --scope agent_messages:read \
  --scope prompts:respond --scope approvals:respond \
  --role primary \
  --tool filesystem.list --tool filesystem.read --tool filesystem.search \
  --tool git.status --tool git.diff --tool user.ask \
  --tool plan.create --tool plan.update --tool plan.show \
  --credential-keyring-service dev.obscuritylabs.colossus.vscode \
  --credential-keyring-account local
```

This grants the listed read tools, local Plan records, and questions. Add only the exact tools needed for
your workflow when enrolling; for implementation, that can include `patch.preview`,
`patch.apply`, and `shell.run`. Runtime policy and sandbox restrictions still apply.
Inspect the configured catalog with `colossus tools list` before choosing the ceiling.

Research with workspace evidence uses the example's `filesystem.search` grant. For
Web or MCP evidence, also enroll the application with `--tool web.search` or
`--tool mcp.call`, respectively. Configure the worker's research search route and
normal MCP allowed tools before selecting those sources. Research inherits `allowedTools`
and uses a tool-capable `research_worker` model to choose calls. Optional
[research projections](../../docs/reference/configuration/mcp.md#research-templates)
override that selection per server; selecting a source does not grant tool access.

Enrollment stores the bearer directly in the OS keyring and prints non-secret
`instance_id` and `certificate_sha256`. Keep those values independently of the discovery
directory. Start the worker with the same workspace, configuration, and public API
directory:

```sh
colossus --workspace /absolute/path/to/repository worker \
  --public-api-dir /absolute/private/colossus-vscode-api
```

Open Colossus in the secondary sidebar (or select **Colossus: Connect Worker** from the
Command Palette) and select **Connect worker**. Choose the workspace
(if multiple folders are open), the discovery directory, the independent enrollment ID
and fingerprint, the OS-keyring service/account, and the enrolled model role. Review
the native confirmation. The extension stores the connection's trust anchors in VS
Code SecretStorage, separately from discovery metadata. It reads the bearer directly
from the native credential store. Tokens are never entered through the sidebar,
settings, files, arguments, or environment variables.

Linux requires an unlocked Secret Service store, matching the worker's persistent
keyring backend; the extension does not fall back to kernel-memory credentials. On
macOS, the keychain may ask whether VS Code can access a CLI-created entry. Generic
same-user credential stores retain the limitations described in the SDK guide.

## Work in the editor

Use the chat header's settings button or **Colossus: Open Settings** to open a full
editor tab. **Global → Defaults** changes the default run mode and send shortcut;
**Global → Appearance** controls tool activity, the Editor (Dark+), Colossus blue, and Hacker (TUI)
dark palettes, and links to VS Code's theme settings. Desktop offers the same
neutral dark and Hacker palettes in Global → Appearance → Dark palette. Hacker
uses the TUI’s green prompts, pale-green text, cyan tools, and amber/red statuses
on near-black surfaces. Palette choices are saved per host and do not change VS
Code’s native editor colors. Light and high contrast keep their own colors.
These preferences are stored in VS Code's user settings and update the chat immediately.
**Workspace → Connections** connects, disconnects, forgets trust settings, and opens
diagnostics. Runtime and Access explain the connected worker's ownership; they do not
edit providers, credentials, MCP servers, policy, or sandbox settings.

Choose **Research** in the composer to expose **Research depth** (Quick, Standard, or
Deep) and **Evidence sources** (This Workspace, Web, or **MCP connections**). Research
starts with Standard depth and workspace evidence. Select MCP connections to collect
from the worker's enabled MCP tools or explicit research projections. At least one source is required;
depth and source choices survive webview reloads. The Research option is disabled
when the worker does not advertise support. Existing run history, interactions,
streaming, and cancellation also apply to research runs.

For existing installs, VS Code may retain a view's old location. Right-click its title,
choose **Move View**, and select **Secondary Side Bar**, or drag the Colossus view there.
Enable **View: Toggle Secondary Side Bar Visibility** if the right sidebar is hidden.
Keep **Work** in the secondary sidebar and **Workspace** in the primary sidebar.
Tool calls appear between the task and response in a compact, open progress thread.
Each call updates in place from the worker’s recorded lifecycle; expand a row to
inspect its states, timestamps, and policy-released input/output. Unknown outcomes
stay explicit. Progress follows the bottom of the conversation while you are
there and preserves your position when you scroll up. The tool-activity setting
hides these threads. Reopening a session reads recorded progress without replaying
actions; an unavailable or oversized historical feed shows a notice while keeping
the saved response. The extension retains up to 100 calls across the latest 20
turns, eight lifecycle entries per call, and bounded detail previews.

The chat history button focuses the left workspace view. **Colossus: Open Workspace**
also reveals it from the Command Palette, including when disconnected.

Version 0.1.7 corrects the left view container ID. VS Code rejects dots in container
IDs; earlier 0.1.6 builds consequently placed Workspace inside Explorer without its
Colossus activity-bar icon. Install the update and reload the window. Build and
packaging checks now validate container IDs and view bindings before producing a VSIX.

- Start with **Plan**, or choose **Execute** for runtime-authorized implementation.
- Use **Add selection** or **Add file** to attach explicit editor snapshots. Excerpts
  are limited to 64 KiB each, eight excerpts, and 96 KiB total. Nothing is attached
  automatically. Execute prompts to save dirty workspace documents or switch to Plan.
- Use **Review and respond** for pending interactions. Approval details preserve exact
  argument boundaries. **Allow once** and **Reject** are native host decisions bound to
  the current run, obligation, revision token, and request binding.
- **Stop** requests cancellation once; the run remains busy until terminal evidence
  arrives. While idle or after observation pauses, **Disconnect** stops observation and leaves the independent worker running.
- Connect again to recover the remembered conversation, or choose a recent conversation
  from the left Sessions view. **Load older history** reads another bounded page of
  caller-owned runs. Chat shows the latest 20 runs in that session
  with their released results and task summaries. Full prompts remain in canonical
  runtime state; the current public API exposes run titles rather than session message
  history. Prompts entered during this extension session remain visible in memory.
  **Reconnect worker** rereads the trusted worker’s discovery endpoint, restores the
  selected conversation and unanswered interactions, and resumes its durable feed.
  It can recover a paused active run after a worker restart changes the port; it
  does not create another run, cancel the run, or replay an interaction response.
- **Plans** lists canonical plan IDs, exact revisions, and lifecycle states from loaded
  terminal run metadata. It never infers a saved plan from assistant prose. Inspection
  shows the released plan output when a matching Plan run is in loaded history.
- **Inspect session** and individual run rows open a snapshot with durable identities,
  lifecycle, timestamps, feed sequence, pending interaction count, and released output.
  **Refresh state** explicitly reads the worker again. The **Session activity** tab uses
  the authenticated `sessions.activity` capability and shows the latest 25 released
  activities plus projection freshness. Unsupported workers show availability clearly.
- **Runtime** lists capabilities available to this enrollment. Full goals, memory,
  workflows, process supervision, plugins, session topology, and plan continuation
  controls are not implemented in the extension yet. Some require additional public
  worker API coverage. Settings do not configure Desktop's private sidecar.
- If creation or a response has an uncertain outcome, inspect/reconcile the worker's
  durable state before another task. Effectful requests are never replayed automatically.

**Colossus: Forget Connection** removes the protected local connection profile while
idle. Revoke the application's credential separately in worker administration; forgetting
the profile does not revoke it or delete canonical evidence.

## Connection troubleshooting

Start the worker separately from enrollment. Enrollment is a one-time offline operation;
`DestinationExists` means its keyring destination is already populated, not that a worker
is running. Keep `colossus worker --public-api-dir /absolute/private/colossus-vscode-api`
running while connecting. Select that exact discovery directory in the extension.
On macOS, use Cmd+Shift+G in the folder picker to open a hidden directory.

Connection failures identify the stage: discovery, certificate identity, keyring,
authenticated handshake, saved profile, or history. Use **Colossus: Show Connection
Diagnostics** (or the error notification's **Show diagnostics** button) to inspect fixed
stage/category records in the native Output panel. The records contain no tokens, raw
exceptions, metadata, connection paths, or enrollment anchors. A missing endpoint file
usually means the worker is stopped or the selected discovery folder is incorrect.
An identity mismatch requires checking the saved values against trusted enrollment output;
discovery files never authorize replacing a saved trust anchor.

`keyring: failed (credential-not-found)` means the native store returned no item at
the saved service/account. Store-access errors are reported separately; this is not a
worker TLS failure. Use **Colossus: Configure Credential Location** (or **Workspace →
Connections → Credential location**) to inspect and correct those two names in native
input dialogs. This retains the worker identity, certificate pin, role, and workspace
binding and does not read a credential or reconnect automatically. Cancel either dialog
to keep the saved profile intact.

On macOS, check item metadata without printing its password:

```sh
security find-generic-password -s dev.obscuritylabs.colossus.vscode -a local
```

If that item is absent, omit `-a local` to look for an enrollment under another account.
If the service has no item, stop the worker and repeat the enrollment command above with
the original configuration and grants, then start the worker separately and reconnect.
Starting a worker alone never creates a missing enrollment credential. If enrollment
reports `DestinationExists`, inspect the configured keychain/store and service/account
before replacing or deleting an entry.

Version 0.1.3 configures the pinned end-entity certificate as a direct TLS trust anchor
for Electron's certificate verifier. If an older extension reaches `handshake` and
fails while the worker is listening, install 0.1.3 or rebuild with the updated SDK.
Certificate, hostname, instance, and TLS 1.3 validation remain enforced; this fix does
not require deleting or re-enrolling credentials.

To reset the extension's credential on macOS, stop the worker and revoke its credential
using the `credential_id` printed by enrollment. Run administration with the same workspace,
configuration, and discovery directory as that enrollment:

```sh
colossus --workspace /absolute/path/to/repository worker \
  --public-api-dir /absolute/private/colossus-vscode-api \
  --revoke-credential <credential_id_from_enrollment>
security delete-generic-password \
  -s dev.obscuritylabs.colossus.vscode -a local
```

Use **Colossus: Forget Connection**, then enroll again with the command above and
retain its newly printed ID and fingerprint. Start the worker separately and reconnect.
This resets only that extension enrollment and saved connection; the worker identity,
provider credentials, configuration, and run history are retained.

## Validation

```sh
npm run check
npm run build
npm exec -- playwright install chromium
npm run test:browser
npm run test:live
npm audit --audit-level=high
npm run package
```

Build `cargo build --locked -p colossus-cli --example sdk_ephemeral_local` from the
repository root before `test:live`. This acceptance test uses the real worker and
offline echo provider with disposable state and an anonymous credential pipe. It
restores any existing generated TypeScript live runner after testing. It proves run
creation, streaming, released history, session recovery, and a follow-up in the same
conversation without touching an
enrolled OS-keyring entry.

On Unix, run the same acceptance flow in an installed Electron runtime with
`npm run test:live -- --electron /absolute/path/to/electron`. Its Node mode uses the
same credential pipe and disposable worker. CI repeats the flow in checksum-pinned
Electron 42.10.0, including the worker's CA=false certificate, because regular Node's
OpenSSL verifier alone does not establish compatibility with VS Code's BoringSSL.

The transport tests use real pinned TLS/gRPC with a deterministic application fixture.
They cover credential isolation, streaming, disconnect recovery, native-review requests,
one-use answers, cancellation, uncertain outcomes, and bounded retries of admission-limited
history reads. Browser tests check the sidebar and settings,
keyboard/accessibility basics, narrow widths, draft retention, and literal rendering of
model-authored content. They do not automate native credential access or VS Code's
on-device dialogs. Validate those with an enrolled worker on the intended platform.

Bundled runtime startup, shared Desktop conversation handoff, structured patch proposals,
inline editing, model selection, and richer workflow/plugin panels are follow-up work.

## Review UI previews

After building and running the tests, run `npm run preview:capture` with the installed
Playwright Chromium browser. It writes a full screenshot gallery and standalone
renderer previews to `artifacts/review/`. These use explicit sample data; they are
browser captures of the actual extension renderers, not native VS Code screenshots
or evidence that the extension connected to a worker.

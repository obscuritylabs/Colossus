import { createHash, randomBytes, randomUUID } from "node:crypto";
import { realpath } from "node:fs/promises";
import { isAbsolute, relative, sep } from "node:path";
import * as vscode from "vscode";
import {
  connectWorker,
  connectionStep,
  parseProfile,
  safeError,
  workspaceIdentity,
  type ConnectionProfile,
} from "./connection.js";
import { WorkController } from "./controller.js";
import {
  ConnectionError,
  UserError,
  type ConnectionDiagnostic,
} from "./errors.js";
import { parseAction, type WorkView } from "./model.js";
import {
  parseSettingsAction,
  preferenceSaveError,
  PREFERENCE_KEYS,
  readPreferences,
  type SettingsView,
} from "./settings.js";
import type {
  Interaction,
  RespondInteractionRequest,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/agent_run";

const KEYRING_SERVICE = "dev.obscuritylabs.colossus.vscode";
const KEYRING_ACCOUNT = "local";

function storageKey(path: string) {
  return `colossus.connection.v1.${createHash("sha256").update(path).digest("hex")}`;
}

export function activate(context: vscode.ExtensionContext) {
  const diagnostics = vscode.window.createOutputChannel("Colossus Connection");
  context.subscriptions.push(diagnostics);
  const report = (event: ConnectionDiagnostic) => {
    diagnostics.appendLine(
      `${event.stage}: ${event.state}${event.reason ? ` (${event.reason})` : ""}`,
    );
  };
  const reviews = new Map<string, string>();
  context.subscriptions.push(
    vscode.workspace.registerTextDocumentContentProvider("colossus-review", {
      provideTextDocumentContent: (uri) => reviews.get(uri.toString()) ?? "",
    }),
  );
  context.subscriptions.push(
    vscode.workspace.onDidCloseTextDocument((document) => {
      if (document.uri.scheme === "colossus-review")
        reviews.delete(document.uri.toString());
    }),
  );
  let folder: vscode.WorkspaceFolder | undefined;
  let profile: ConnectionProfile | undefined;
  let webview: vscode.WebviewView | undefined;
  let explorerView: vscode.WebviewView | undefined;
  let inspectionPanel: vscode.WebviewPanel | undefined;
  let selectedInspection:
    | {
        id: string;
        kind: "run" | "plan";
        owner: WorkController;
        generation: number;
      }
    | undefined;
  let settingsPanel: vscode.WebviewPanel | undefined;
  let preferenceError = "";
  let connecting = false;
  let configuringCredential = false;
  let controller = makeController("Select a workspace");
  let publishTimer: ReturnType<typeof setTimeout> | undefined;
  let lastView: WorkView = controller.view;
  const status = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Left,
    20,
  );
  status.command = "colossus.work.focus";
  status.text = "Colossus";
  status.tooltip = "Open Colossus Work";
  status.show();
  context.subscriptions.push(status);

  function preferences() {
    const config = vscode.workspace.getConfiguration("colossus");
    return readPreferences((key) => config.get(key));
  }

  function publishViews() {
    void webview?.webview.postMessage({
      type: "state",
      view: lastView,
      preferences: preferences(),
    });
    void explorerView?.webview.postMessage({
      type: "state",
      view: lastView,
      preferences: preferences(),
    });
    if (
      selectedInspection &&
      (selectedInspection.owner !== controller ||
        selectedInspection.generation !== controller.connectionGeneration)
    )
      selectedInspection = undefined;
    void inspectionPanel?.webview.postMessage({
      type: "inspection",
      preferences: preferences(),
      view: lastView.inspection,
      loading: lastView.inspectionLoading,
      connected: lastView.connected,
      error: lastView.error,
    });
    const settings: SettingsView = {
      preferences: preferences(),
      preferenceError,
      workspace: lastView.workspace,
      connected: lastView.connected,
      connecting,
      busy: lastView.busy,
      reconnectable: controller.canReconnect,
      hasSavedConnection: !!profile,
      version: lastView.version,
      role: profile?.role ?? "",
      error: lastView.error,
    };
    void settingsPanel?.webview.postMessage({
      type: "settings",
      view: settings,
    });
  }

  function configureWebview(
    view: vscode.Webview,
    page: "work" | "settings" | "explorer" | "inspector",
  ) {
    const assets = vscode.Uri.joinPath(context.extensionUri, "dist");
    view.options = { enableScripts: true, localResourceRoots: [assets] };
    const nonce = randomBytes(24).toString("base64");
    const asset = (name: string) =>
      view.asWebviewUri(vscode.Uri.joinPath(assets, name));
    const script = page === "work" ? "webview" : page;
    const extraCss =
      page === "settings"
        ? "settings.css"
        : page === "work"
          ? ""
          : "workspace.css";
    view.html = `<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${view.cspSource}; script-src 'nonce-${nonce}'; img-src ${view.cspSource}; connect-src 'none'; base-uri 'none'; form-action 'none'"><link rel="stylesheet" href="${asset("theme.css")}"><link rel="stylesheet" href="${asset("style.css")}"><link rel="stylesheet" href="${asset("composer.css")}"><link rel="stylesheet" href="${asset("select.css")}">${page === "settings" ? `<link rel="stylesheet" href="${asset("settings-frame.css")}">` : ""}${extraCss ? `<link rel="stylesheet" href="${asset(extraCss)}">` : ""}<title>Colossus ${page}</title></head><body data-colossus-mark="${asset("colossus-mark.svg")}"><div id="app"></div><script nonce="${nonce}" src="${asset(`${script}.js`)}"></script></body></html>`;
  }

  async function openInspector(id: string, kind: "run" | "plan") {
    await guard();
    if (
      !(kind === "run" ? controller.view.runs : controller.view.plans).some(
        (item) => item.id === id,
      )
    )
      throw new UserError("Refresh and select a listed run or plan.");
    selectedInspection = {
      id,
      kind,
      owner: controller,
      generation: controller.connectionGeneration,
    };
    if (!inspectionPanel) {
      const panel = vscode.window.createWebviewPanel(
        "colossus.inspector",
        "Colossus Inspector",
        vscode.ViewColumn.Active,
        {},
      );
      inspectionPanel = panel;
      panel.iconPath = vscode.Uri.joinPath(
        context.extensionUri,
        "media",
        "colossus.svg",
      );
      configureWebview(panel.webview, "inspector");
      const listener = panel.webview.onDidReceiveMessage((value) => {
        if (
          !value ||
          typeof value !== "object" ||
          Object.keys(value).length !== 1
        )
          return;
        if (value.type === "ready") {
          publishViews();
          return;
        }
        if (value.type !== "refreshInspection") return;
        void run(async () => {
          await guard();
          const selected = selectedInspection;
          if (
            !selected ||
            selected.owner !== controller ||
            selected.generation !== controller.connectionGeneration
          )
            throw new UserError("Select a listed run or plan again.");
          await controller.inspect(selected.id, selected.kind);
        });
      });
      panel.onDidDispose(
        () => {
          listener.dispose();
          if (inspectionPanel === panel) {
            inspectionPanel = undefined;
            selectedInspection = undefined;
          }
        },
        undefined,
        context.subscriptions,
      );
      context.subscriptions.push(panel);
    } else inspectionPanel.reveal(vscode.ViewColumn.Active);
    await controller.inspect(id, kind);
  }

  function openSettings() {
    if (settingsPanel) {
      settingsPanel.reveal(vscode.ViewColumn.Active);
      return;
    }
    const panel = vscode.window.createWebviewPanel(
      "colossus.settings",
      "Colossus Settings",
      vscode.ViewColumn.Active,
      {},
    );
    settingsPanel = panel;
    panel.iconPath = vscode.Uri.joinPath(
      context.extensionUri,
      "media",
      "colossus.svg",
    );
    configureWebview(panel.webview, "settings");
    const listener = panel.webview.onDidReceiveMessage((value) => {
      const action = parseSettingsAction(value);
      if (!action) return;
      if (action.type === "setPreference") {
        void savePreference(action);
        return;
      }
      void run(async () => {
        switch (action.type) {
          case "ready":
            publishViews();
            break;
          case "openWork":
            await vscode.commands.executeCommand("colossus.work.focus");
            break;
          case "openThemeSettings":
            try {
              await vscode.commands.executeCommand(
                "workbench.action.selectTheme",
              );
            } catch {
              // A workbench command failure is not a worker failure. Keep it out
              // of run state and never expose raw command/extension errors.
              await vscode.window.showErrorMessage(
                "VS Code’s theme picker could not be opened. Use Preferences: Color Theme from the Command Palette.",
              );
            }
            break;
          case "openUserSettings":
            await openUserSettings();
            break;
          default:
            await commands[`colossus.${action.type}`]?.();
            publishViews();
            break;
        }
      });
    });
    panel.onDidDispose(
      () => {
        listener.dispose();
        if (settingsPanel === panel) settingsPanel = undefined;
      },
      undefined,
      context.subscriptions,
    );
    context.subscriptions.push(panel);
  }

  async function savePreference(
    action: Extract<
      ReturnType<typeof parseSettingsAction>,
      { type: "setPreference" }
    >,
  ) {
    try {
      const config = vscode.workspace.getConfiguration("colossus");
      const key = PREFERENCE_KEYS[action.name];
      if (config.inspect(key)?.defaultValue === undefined)
        throw new UserError("This is not a registered configuration.");
      await config.update(key, action.value, vscode.ConfigurationTarget.Global);
      preferenceError = "";
    } catch (error) {
      preferenceError = preferenceSaveError(action.name, error);
    }
    // Publish the values actually saved by VS Code, including rollback after a
    // rejected write. A presentation preference never mutates worker/run state.
    publishViews();
  }

  async function openUserSettings() {
    try {
      await vscode.commands.executeCommand("workbench.action.openSettingsJson");
    } catch {
      await vscode.window.showErrorMessage(
        "VS Code’s user settings could not be opened. Use Preferences: Open User Settings (JSON) from the Command Palette.",
      );
    }
  }

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("colossus")) publishViews();
    }),
  );

  function makeController(label: string) {
    return new WorkController(label, {
      publish: (view) => {
        lastView = view;
        status &&
          (status.text = view.busy ? "$(sync~spin) Colossus" : "Colossus");
        // Bound token-stream rendering without changing the ordered runtime feed.
        if (publishTimer === undefined)
          publishTimer = setTimeout(() => {
            publishTimer = undefined;
            publishViews();
          }, 50);
      },
      remember: async (sessionId) => {
        if (profile)
          await context.workspaceState.update(
            `session:${profile.instanceId}:${profile.workspacePath}`,
            sessionId,
          );
      },
      interaction: (interaction) =>
        presentInteraction(interaction, async (text) => {
          const uri = vscode.Uri.parse(
            `colossus-review:/approvals/${randomUUID()}.txt`,
          );
          reviews.set(uri.toString(), text);
          await vscode.window.showTextDocument(
            await vscode.workspace.openTextDocument(uri),
            { preview: true },
          );
        }),
    });
  }

  async function chooseFolder() {
    if (!vscode.workspace.isTrusted)
      throw new UserError("Trust this workspace before using Colossus.");
    const folders =
      vscode.workspace.workspaceFolders?.filter(
        (f) => f.uri.scheme === "file",
      ) ?? [];
    if (!folders.length)
      throw new UserError("Open a local workspace folder first.");
    if (
      folder &&
      folders.some((f) => f.uri.toString() === folder?.uri.toString())
    )
      return folder;
    if (folders.length === 1) return folders[0];
    return (
      await vscode.window.showQuickPick(
        folders.map((f) => ({
          label: f.name,
          description: f.uri.fsPath,
          folder: f,
        })),
        { title: "Choose the Colossus workspace", ignoreFocusOut: true },
      )
    )?.folder;
  }

  async function configure(
    selected: vscode.WorkspaceFolder,
  ): Promise<ConnectionProfile | undefined> {
    const workspace = await workspaceIdentity(selected.uri.fsPath);
    const directories = await vscode.window.showOpenDialog({
      title: "Select the enrolled worker's public API directory",
      canSelectFiles: false,
      canSelectFolders: true,
      canSelectMany: false,
      openLabel: "Use discovery directory",
    });
    if (!directories?.[0] || directories[0].scheme !== "file") return;
    const discoveryDirectory = await realpath(directories[0].fsPath);
    const instanceId = await vscode.window.showInputBox({
      title: "Colossus enrollment: instance ID",
      prompt:
        "Paste instance_id from the trusted enrollment output, separately from endpoint.json.",
      ignoreFocusOut: true,
      validateInput: (value) =>
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(
          value,
        )
          ? undefined
          : "Enter the enrollment's lowercase UUID.",
    });
    if (!instanceId) return;
    const certificateSha256 = await vscode.window.showInputBox({
      title: "Colossus enrollment: certificate fingerprint",
      prompt:
        "Paste certificate_sha256 from the enrollment output. Do not obtain it from the discovery files.",
      ignoreFocusOut: true,
      validateInput: (value) =>
        /^[0-9a-f]{64}$/u.test(value)
          ? undefined
          : "Enter 64 lowercase hexadecimal characters.",
    });
    if (!certificateSha256) return;
    const identifier = (title: string, value: string) =>
      vscode.window.showInputBox({
        title,
        value,
        ignoreFocusOut: true,
        validateInput: (text) =>
          /^[A-Za-z0-9._:-]{1,128}$/u.test(text)
            ? undefined
            : "Use 1–128 letters, numbers, dots, colons, underscores, or hyphens.",
      });
    const keyringService = await identifier(
      "Enrolled OS-keyring service",
      KEYRING_SERVICE,
    );
    if (!keyringService) return;
    const keyringAccount = await identifier(
      "Enrolled OS-keyring account",
      KEYRING_ACCOUNT,
    );
    if (!keyringAccount) return;
    const role = await vscode.window.showInputBox({
      title: "Colossus model role",
      value: "primary",
      prompt:
        "This role must be included in the application's enrollment grant.",
      ignoreFocusOut: true,
      validateInput: (value) =>
        /^[A-Za-z0-9._-]{1,64}$/u.test(value)
          ? undefined
          : "Enter a configured role name.",
    });
    if (!role) return;
    const confirmation = await vscode.window.showWarningMessage(
      "Trust this enrolled Colossus worker for this workspace?",
      {
        modal: true,
        detail: `Workspace: ${workspace.path}\nInstance: ${instanceId}\nCertificate SHA-256: ${certificateSha256}\n\nThe worker must have been enrolled for this workspace. Its runtime policy and application grant govern execution.`,
      },
      "Trust enrolled worker",
    );
    if (confirmation !== "Trust enrolled worker") return;
    return parseProfile({
      schemaVersion: 1,
      workspacePath: workspace.path,
      workspaceIdentity: workspace.identity,
      discoveryDirectory,
      instanceId,
      certificateSha256,
      keyringService,
      keyringAccount,
      role,
    });
  }

  async function guard() {
    if (
      !vscode.workspace.isTrusted ||
      !folder ||
      !profile ||
      !vscode.workspace.workspaceFolders?.some(
        (f) => f.uri.toString() === folder?.uri.toString(),
      )
    )
      throw new UserError("Select and connect a trusted workspace.");
    const current = await workspaceIdentity(folder.uri.fsPath);
    if (
      current.path !== profile.workspacePath ||
      current.identity !== profile.workspaceIdentity
    ) {
      controller.detach();
      throw new UserError(
        "The workspace was replaced. Reconnect and enroll the folder again.",
      );
    }
  }

  async function configureCredentialLocation() {
    if (connecting || configuringCredential || controller.view.busy)
      throw new UserError(
        "Wait for the current connection or run before changing the credential location.",
      );
    configuringCredential = true;
    try {
      const selected = await chooseFolder();
      if (!selected) return;
      const canonical = await workspaceIdentity(selected.uri.fsPath);
      const stored = await connectionStep(
        "saved-profile",
        () => context.secrets.get(storageKey(canonical.path)),
        report,
      );
      if (!stored)
        throw new UserError(
          "Connect and configure this workspace's worker before changing its credential location.",
        );
      const enrolled = await connectionStep(
        "saved-profile",
        async () => parseProfile(JSON.parse(stored)),
        report,
      );
      if (
        enrolled.workspacePath !== canonical.path ||
        enrolled.workspaceIdentity !== canonical.identity
      )
        throw new UserError(
          "The workspace has changed. Reconnect and enroll the original folder.",
        );
      const identifier = (title: string, value: string, prompt: string) =>
        vscode.window.showInputBox({
          title,
          value,
          prompt,
          ignoreFocusOut: true,
          validateInput: (text) =>
            /^[A-Za-z0-9._:-]{1,128}$/u.test(text)
              ? undefined
              : "Use 1–128 letters, numbers, dots, colons, underscores, or hyphens.",
        });
      const service = await identifier(
        "Colossus credential: OS-keyring service",
        enrolled.keyringService,
        "The saved service name is shown above. Match --credential-keyring-service from enrollment; enter the name, not the credential.",
      );
      if (!service) return;
      const account = await identifier(
        "Colossus credential: OS-keyring account",
        enrolled.keyringAccount,
        "Match credential_keyring_account from enrollment, or --credential-keyring-account. This changes only the lookup location for the already trusted worker.",
      );
      if (!account) return;
      // Native inputs can nominate identifiers only. Existing independent worker trust
      // anchors and the enrolled role are retained; the renderer supplies no selectors.
      const updated = parseProfile({
        ...enrolled,
        keyringService: service,
        keyringAccount: account,
      });
      const current = await workspaceIdentity(selected.uri.fsPath);
      if (
        !vscode.workspace.isTrusted ||
        !vscode.workspace.workspaceFolders?.some(
          (f) => f.uri.toString() === selected.uri.toString(),
        ) ||
        current.identity !== enrolled.workspaceIdentity ||
        current.path !== enrolled.workspacePath
      )
        throw new UserError(
          "The workspace changed while configuring the credential. Reopen the original workspace and try again.",
        );
      await connectionStep(
        "saved-profile",
        () =>
          context.secrets.store(
            storageKey(canonical.path),
            JSON.stringify(updated),
          ),
        report,
      );
      controller.detach();
      folder = selected;
      profile = updated;
      controller.view.error = "";
      controller.view.status =
        "Credential location saved. Reconnect the worker.";
      controller.publish();
    } finally {
      configuringCredential = false;
    }
  }

  async function connect() {
    if (connecting || configuringCredential || !controller.canReconnect) return;
    const recovering = controller.view.busy;
    connecting = true;
    publishViews();
    try {
      const selected = recovering ? folder : await chooseFolder();
      if (!selected) return;
      const canonical = await workspaceIdentity(selected.uri.fsPath);
      const stored = await connectionStep(
        "saved-profile",
        () => context.secrets.get(storageKey(canonical.path)),
        report,
      );
      const enrolled = recovering
        ? profile
        : stored
          ? await connectionStep(
              "saved-profile",
              async () => parseProfile(JSON.parse(stored)),
              report,
            )
          : await configure(selected);
      if (!enrolled) return;
      folder = selected;
      profile = enrolled;
      controller.detach();
      controller = makeController(selected.name);
      controller.view.connecting = true;
      controller.view.status = "Connecting to worker";
      controller.publish();
      const client = await connectWorker(
        enrolled,
        async (service, account) => {
          // Native keyring access stays in the extension host. Never send credentials to the webview.
          const { AsyncEntry } = await import("@napi-rs/keyring");
          return (
            (await new AsyncEntry(service, account, {
              linux: { store: "secret-service" },
            }).getPassword()) ?? null
          );
        },
        report,
      );
      try {
        if (!vscode.workspace.isTrusted)
          throw new UserError("Workspace trust changed.");
        await guard();
        await connectionStep(
          "saved-profile",
          () =>
            context.secrets.store(
              storageKey(enrolled.workspacePath),
              JSON.stringify(enrolled),
            ),
          report,
        );
        const session = context.workspaceState.get<string>(
          `session:${enrolled.instanceId}:${enrolled.workspacePath}`,
          "",
        );
        await connectionStep(
          "history",
          () => controller.attach(client, enrolled.role, session),
          report,
        );
      } catch (error) {
        controller.detach();
        client.close();
        throw error;
      }
    } finally {
      connecting = false;
      controller.view.connecting = false;
      controller.publish();
    }
  }

  async function addContext(selection: boolean) {
    await guard();
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.uri.scheme !== "file" || !profile)
      throw new UserError("Open a file in this workspace.");
    const canonical = await realpath(editor.document.uri.fsPath);
    const path = relative(profile.workspacePath, canonical);
    if (isAbsolute(path) || path === ".." || path.startsWith(`..${sep}`))
      throw new UserError("Select a file inside the connected workspace.");
    if (selection && editor.selection.isEmpty)
      throw new UserError("Select some text first.");
    const text = editor.document.getText(
      selection ? editor.selection : undefined,
    );
    if (Buffer.byteLength(text, "utf8") > 64 * 1024)
      throw new UserError("Choose an excerpt of at most 64 KiB.");
    const lines = selection
      ? `:${editor.selection.start.line + 1}–${editor.selection.end.line + 1}`
      : "";
    controller.setContext({
      label: `${path}${lines} · ${editor.document.isDirty ? "unsaved" : "saved"} · version ${editor.document.version}`,
      text,
    });
    await vscode.commands.executeCommand("colossus.work.focus");
  }

  async function send(text: string, mode: "plan" | "execute") {
    if (configuringCredential)
      throw new UserError(
        "Finish configuring the credential location before sending a task.",
      );
    await guard();
    if (mode === "execute" && folder) {
      const dirty = vscode.workspace.textDocuments.filter(
        (d) =>
          d.isDirty &&
          vscode.workspace.getWorkspaceFolder(d.uri)?.uri.toString() ===
            folder?.uri.toString(),
      );
      if (dirty.length) {
        const choice = await vscode.window.showWarningMessage(
          "Save workspace files before Colossus executes?",
          {
            modal: true,
            detail:
              "The runtime reads files from disk. Saving first keeps your editor buffers and runtime edits in sync.",
          },
          "Save files and execute",
          "Plan instead",
        );
        if (choice === "Plan instead") mode = "plan";
        else if (choice === "Save files and execute") {
          for (const document of dirty)
            if (!(await document.save()))
              throw new UserError("A workspace file could not be saved.");
        } else return;
      }
    }
    await guard();
    await controller.send(text, mode);
  }

  const commands: Record<string, () => Promise<unknown> | unknown> = {
    "colossus.connect": connect,
    "colossus.disconnect": () => {
      if (connecting || configuringCredential || !controller.canReconnect)
        throw new UserError(
          "Wait for the current action or pause observation before disconnecting.",
        );
      controller.detach();
    },
    "colossus.openSettings": openSettings,
    "colossus.openWorkspace": () =>
      vscode.commands.executeCommand("colossus.workspace.focus"),
    "colossus.configureCredential": configureCredentialLocation,
    "colossus.showConnectionLog": () => diagnostics.show(true),
    "colossus.forgetConnection": async () => {
      if (connecting || configuringCredential || controller.view.busy) return;
      const selected = profile ? undefined : await chooseFolder();
      const path =
        profile?.workspacePath ??
        (selected
          ? (await workspaceIdentity(selected.uri.fsPath)).path
          : undefined);
      if (!path) return;
      controller.detach();
      await context.secrets.delete(storageKey(path));
      profile = undefined;
    },
    "colossus.newSession": async () => {
      await guard();
      await controller.newSession();
      await vscode.commands.executeCommand("colossus.work.focus");
      void webview?.webview.postMessage({
        type: "newConversation",
        mode: preferences().defaultMode,
      });
    },
    "colossus.addSelection": () => addContext(true),
    "colossus.addFile": () => addContext(false),
    "colossus.reviewChanges": async () => {
      await guard();
      await vscode.commands.executeCommand("workbench.view.scm");
    },
  };
  for (const [name, command] of Object.entries(commands))
    context.subscriptions.push(
      vscode.commands.registerCommand(name, () => run(command)),
    );

  async function run(action: () => Promise<unknown> | unknown) {
    try {
      await action();
    } catch (error) {
      controller.error(error);
      if (error instanceof ConnectionError) {
        const choice = await vscode.window.showErrorMessage(
          safeError(error),
          "Show diagnostics",
          ...(error.stage === "keyring" ? ["Credential location"] : []),
        );
        if (choice === "Show diagnostics") diagnostics.show(true);
        if (choice === "Credential location")
          await run(configureCredentialLocation);
      } else await vscode.window.showErrorMessage(safeError(error));
    }
  }

  for (const [viewId, page] of [
    ["colossus.work", "work"],
    ["colossus.workspace", "explorer"],
  ] as const)
    context.subscriptions.push(
      vscode.window.registerWebviewViewProvider(viewId, {
        resolveWebviewView(view) {
          if (page === "work") webview = view;
          else explorerView = view;
          configureWebview(view.webview, page);
          const listener = view.webview.onDidReceiveMessage((value) => {
            const action = parseAction(value);
            if (!action) return;
            if (
              page === "explorer" &&
              ![
                "ready",
                "connect",
                "openSettings",
                "newSession",
                "selectSession",
                "refreshSessions",
                "loadMoreSessions",
                "inspectRun",
                "inspectPlan",
                "openWork",
              ].includes(action.type)
            )
              return;
            void run(async () => {
              switch (action.type) {
                case "ready":
                  controller.publish();
                  break;
                case "connect":
                  await connect();
                  break;
                case "disconnect":
                  await commands["colossus.disconnect"]?.();
                  break;
                case "openWorkspace":
                  await commands["colossus.openWorkspace"]?.();
                  break;
                case "openWork":
                  await vscode.commands.executeCommand("colossus.work.focus");
                  break;
                case "inspectRun":
                case "inspectPlan":
                  await openInspector(
                    action.id,
                    action.type === "inspectRun" ? "run" : "plan",
                  );
                  break;
                case "loadMoreSessions":
                  await guard();
                  await controller.refreshSessions(true);
                  break;
                case "openSettings":
                  openSettings();
                  break;
                case "send":
                  await send(action.text, action.mode);
                  break;
                case "addSelection":
                  await addContext(true);
                  break;
                case "addFile":
                  await addContext(false);
                  break;
                case "clearContext":
                  controller.clearContext();
                  break;
                case "reviewChanges":
                  await guard();
                  await vscode.commands.executeCommand("workbench.view.scm");
                  break;
                case "newSession":
                  await commands["colossus.newSession"]?.();
                  break;
                case "selectSession":
                  await guard();
                  await controller.selectSession(action.id);
                  await vscode.commands.executeCommand("colossus.work.focus");
                  break;
                case "respond":
                  await guard();
                  await controller.respond(action.id);
                  break;
                case "stop":
                  await guard();
                  await controller.stop();
                  break;
                case "resume":
                  await guard();
                  await controller.resume();
                  break;
                case "refreshSessions":
                  await guard();
                  await controller.refreshSessions();
                  break;
              }
            });
          });
          view.onDidDispose(() => {
            listener.dispose();
            if (webview === view) webview = undefined;
            if (explorerView === view) explorerView = undefined;
          });
          controller.publish();
        },
      }),
    );

  context.subscriptions.push(
    vscode.workspace.onDidChangeWorkspaceFolders(() => {
      if (
        folder &&
        !vscode.workspace.workspaceFolders?.some(
          (f) => f.uri.toString() === folder?.uri.toString(),
        )
      ) {
        controller.detach();
        folder = undefined;
        profile = undefined;
      }
    }),
  );
  context.subscriptions.push({
    dispose: () => {
      controller.detach();
      if (publishTimer) clearTimeout(publishTimer);
    },
  });
}

async function presentInteraction(
  interaction: Interaction,
  review: (text: string) => Promise<void>,
): Promise<RespondInteractionRequest["response"]> {
  const content = interaction.content;
  if (!interaction.respondableByCaller || !content) return;
  if (content.$case === "approval") {
    const approval = content.value;
    const command = approval.commandContext;
    const detail = [
      approval.reason,
      `Action: ${approval.action}`,
      `Resource: ${approval.resource}`,
      command
        ? `Purpose: ${command.justification}\nWorking directory: ${command.workingDirectory}\nPrepared argument vector:\n${JSON.stringify([command.executable, ...command.arguments], null, 2)}${command.redacted ? "\nCredential-bearing text is redacted." : ""}`
        : "",
    ]
      .filter(Boolean)
      .join("\n\n");
    const choice = await vscode.window.showWarningMessage(
      "Colossus needs approval for this action",
      {
        modal: true,
        detail:
          detail.length <= 16000
            ? detail
            : `${detail.slice(0, 1000)}\n\nOpen Review details to see the full prepared command before approving.`,
      },
      ...(detail.length <= 16000
        ? ["Allow once", "Reject"]
        : ["Review details", "Reject"]),
    );
    let decision = choice;
    if (choice === "Review details") {
      await review(detail);
      decision = await vscode.window.showWarningMessage(
        "Approve the exact action shown in the Colossus review?",
        {
          modal: true,
          detail: `${approval.reason}\nAction: ${approval.action}\nThis approval applies once to the displayed request.`,
        },
        "Allow once",
        "Reject",
      );
    }
    if (decision !== "Allow once" && decision !== "Reject") return;
    return {
      $case: "approvalAnswer",
      value: {
        approved: decision === "Allow once",
        requestHash: approval.requestHash,
      },
    };
  }
  const prompt = content.value;
  const choices = prompt.choices.map((choice) => ({
    label: choice.label,
    choice,
  }));
  if (choices.length) {
    const picked = await vscode.window.showQuickPick(
      [
        ...choices,
        ...(prompt.allowFreeForm
          ? [{ label: "Write an answer…", choice: undefined }]
          : []),
      ],
      { title: prompt.question, ignoreFocusOut: true },
    );
    if (!picked) return;
    if (picked.choice)
      return {
        $case: "promptAnswer",
        value: { answer: { $case: "choice", value: picked.choice } },
      };
  }
  if (!prompt.allowFreeForm) return;
  const answer = await vscode.window.showInputBox({
    title: prompt.question,
    ignoreFocusOut: true,
    validateInput: (value) =>
      value.trim() && Buffer.byteLength(value, "utf8") <= 64 * 1024
        ? undefined
        : "Enter an answer of at most 64 KiB.",
  });
  return answer
    ? {
        $case: "promptAnswer",
        value: { answer: { $case: "freeFormText", value: answer } },
      }
    : undefined;
}

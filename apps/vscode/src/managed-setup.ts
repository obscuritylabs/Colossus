import * as vscode from "vscode";
import { randomUUID } from "node:crypto";
import { UserError } from "./errors.js";
import { workspaceIdentity } from "./connection.js";
import {
  runtimeConfiguration,
  type ManagedSetup,
  type ManagedProvider,
} from "./managed.js";

export const managedKey = (workspace: string) =>
  `colossus.managed.v1:${workspace}`;
const credentialKey = (id: string) => `colossus.provider.v1:${id}`;

export async function managedCredentials(
  context: vscode.ExtensionContext,
  setup: ManagedSetup,
) {
  const credentials: { id: string; secret: string }[] = [];
  for (const provider of setup.providers) {
    if (!provider.credential_id) continue;
    const secret = await context.secrets.get(
      credentialKey(provider.credential_id),
    );
    if (!secret)
      throw new UserError(
        "A saved provider credential is unavailable. Configure Local Runtime to replace it.",
      );
    credentials.push({ id: provider.credential_id, secret });
  }
  return credentials;
}

export async function configureManaged(
  context: vscode.ExtensionContext,
  workspace: string,
): Promise<ManagedSetup | undefined> {
  const selectedWorkspace = await workspaceIdentity(workspace);
  let saved = context.workspaceState.get<ManagedSetup>(managedKey(workspace));
  if (saved && saved.workspaceIdentity !== selectedWorkspace.identity) {
    const selected = await vscode.window.showWarningMessage(
      "Set up Colossus for this replacement folder?",
      {
        modal: true,
        detail: `Workspace: ${workspace}\n\nThe saved folder identity has changed. Provider configuration and development access will be reset for this folder. Previous conversation state stays in its original private partition.`,
      },
      "Set up this folder",
    );
    if (selected !== "Set up this folder") return;
    saved = undefined;
  }
  const setup: ManagedSetup = saved
    ? structuredClone(saved)
    : {
        version: 1,
        workspaceIdentity: selectedWorkspace.identity,
        providers: [],
        models: [],
        primary: "",
        access: "minimal",
      };
  const action = saved
    ? await vscode.window.showQuickPick(
        [
          { label: "Add model", id: "model" },
          { label: "Change default model", id: "default" },
          { label: "Change access", id: "access", description: setup.access },
          { label: "Replace provider credential", id: "credential" },
          { label: "Select this workspace again", id: "workspace" },
        ],
        { title: "Managed Local", ignoreFocusOut: true },
      )
    : { id: "model" };
  if (!action) return;
  let pendingSecret: { id: string; value: string } | undefined;
  let clearedCredential: string | undefined;
  if (action.id === "workspace") {
    const selected = await vscode.window.showWarningMessage(
      "Use this folder for Managed Local?",
      {
        modal: true,
        detail: `Workspace: ${workspace}\n\nColossus will bind to the folder currently at this path and reuse the configured providers. Access resets to Minimal. A replacement folder receives its own private state partition.`,
      },
      "Select this workspace",
    );
    if (selected !== "Select this workspace") return;
    delete setup.nativeWorkspaceIdentity;
    setup.workspaceIdentity = selectedWorkspace.identity;
    setup.access = "minimal";
  } else if (action.id === "access") {
    const selected = await vscode.window.showQuickPick(
      [
        {
          label: "Minimal",
          value: "minimal" as const,
          description: "Chat without workspace tools",
        },
        {
          label: "Development",
          value: "development" as const,
          description:
            "Read, edit, Git, plans, and shell with native approvals",
        },
      ],
      { title: "Managed Local access", ignoreFocusOut: true },
    );
    if (!selected) return;
    if (selected.value === "development" && setup.access !== "development") {
      const confirmed = await vscode.window.showWarningMessage(
        "Allow development tools for this workspace?",
        {
          modal: true,
          detail: `Workspace: ${workspace}\n\nColossus may read and edit files, inspect Git, create plans, and run shell commands. Full access can reach files and network resources outside this workspace. Runtime policy and native approval dialogs still apply.`,
        },
        "Allow development tools",
      );
      if (confirmed !== "Allow development tools") return;
    }
    setup.access = selected.value;
  } else if (action.id === "default") {
    const selected = await vscode.window.showQuickPick(
      setup.models.map((model) => ({
        label: model.model,
        description: model.provider_profile,
        model,
      })),
      { title: "Default model", ignoreFocusOut: true },
    );
    if (!selected) return;
    setup.primary = selected.model.profile;
  } else if (action.id === "credential") {
    const selected = await vscode.window.showQuickPick(
      setup.providers
        .filter((provider) => provider.kind !== "echo")
        .map((provider) => ({ label: provider.profile, provider })),
      { title: "Provider credential", ignoreFocusOut: true },
    );
    if (!selected) return;
    const value = await vscode.window.showInputBox({
      title: "Replace provider API key",
      password: true,
      ignoreFocusOut: true,
      prompt:
        "Stored only in VS Code SecretStorage. Leave empty for a server without authentication.",
    });
    if (value === undefined) return;
    const id = selected.provider.credential_id ?? randomUUID();
    if (!value && selected.provider.credential_id)
      clearedCredential = selected.provider.credential_id;
    selected.provider.credential_id = value ? id : null;
    if (value) pendingSecret = { id, value };
  } else {
    if (setup.models.length >= 64)
      throw new UserError("Managed Local supports up to 64 model profiles.");
    const selected = await vscode.window.showQuickPick(
      [
        ...setup.providers.map((provider) => ({
          label: provider.profile,
          description: provider.kind,
          provider,
          protocol: provider.kind,
        })),
        {
          label: "New Responses provider",
          description: "Responses protocol",
          protocol: "open_ai_responses" as const,
        },
        {
          label: "New Anthropic provider",
          description: "Native Messages protocol",
          protocol: "anthropic_messages" as const,
        },
        {
          label: "New compatible provider",
          description: "Chat Completions protocol",
          protocol: "open_ai_compatible" as const,
        },
        {
          label: "Offline self-test",
          description: "Echo only; no AI provider or credential",
          protocol: "echo" as const,
        },
      ],
      { title: "Choose a model provider", ignoreFocusOut: true },
    );
    if (!selected) return;
    let provider: ManagedProvider;
    if ("provider" in selected) provider = selected.provider;
    else {
      if (setup.providers.length >= 16)
        throw new UserError("Managed Local supports up to 16 providers.");
      let profile = `offline-${setup.providers.length + 1}`;
      let baseUrl: string | undefined;
      if (selected.protocol !== "echo") {
        baseUrl = await vscode.window.showInputBox({
          title: "Provider API base URL",
          ...(selected.protocol === "anthropic_messages"
            ? { value: "https://api.anthropic.com/v1" }
            : {}),
          ignoreFocusOut: true,
          prompt:
            "Use the provider's versioned API URL. HTTPS is required except on loopback.",
          validateInput: (value) => {
            try {
              const url = new URL(value);
              return !url.username &&
                !url.password &&
                !url.search &&
                !url.hash &&
                (url.protocol === "https:" ||
                  (url.protocol === "http:" &&
                    ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname)))
                ? undefined
                : "Use an HTTPS API URL, or a loopback HTTP URL, without credentials or query parameters.";
            } catch {
              return "Enter an absolute API URL.";
            }
          },
        });
        if (!baseUrl) return;
        const suggested =
          new URL(baseUrl).hostname
            .replace(/[^A-Za-z0-9._-]/gu, "")
            .slice(0, 64) || "provider";
        const name = await vscode.window.showInputBox({
          title: "Provider name",
          value: setup.providers.some(
            (provider) => provider.profile === suggested,
          )
            ? `${suggested}-${setup.providers.length + 1}`
            : suggested,
          ignoreFocusOut: true,
          prompt: "This name appears in the model picker.",
          validateInput: (value) =>
            /^[A-Za-z0-9._-]{1,64}$/u.test(value) &&
            !setup.providers.some((provider) => provider.profile === value)
              ? undefined
              : "Use a unique name with 1–64 letters, numbers, dots, underscores, or hyphens.",
        });
        if (!name) return;
        profile = name;
        const value = await vscode.window.showInputBox({
          title: "Provider API key",
          password: true,
          ignoreFocusOut: true,
          prompt:
            "Stored only in VS Code SecretStorage. Leave empty for a server without authentication.",
        });
        if (value === undefined) return;
        if (value) pendingSecret = { id: randomUUID(), value };
      }
      provider = {
        profile,
        kind: selected.protocol,
        base_url: baseUrl ?? null,
        credential_id: pendingSecret?.id ?? null,
        timeout_ms: 300000,
      };
      setup.providers.push(provider);
    }
    const model =
      provider.kind === "echo"
        ? "echo"
        : await vscode.window.showInputBox({
            title: "Model identifier",
            ignoreFocusOut: true,
            validateInput: (value) =>
              value.trim() && value.length <= 256 && !/[\r\n\0]/u.test(value)
                ? undefined
                : "Enter the provider's model identifier (up to 256 characters).",
          });
    if (!model) return;
    const integer = async (title: string, value: number) =>
      vscode.window.showInputBox({
        title,
        value: String(value),
        ignoreFocusOut: true,
        validateInput: (text) =>
          /^[0-9]+$/u.test(text) &&
          Number.isSafeInteger(Number(text)) &&
          Number(text) > 0 &&
          Number(text) <= 10000000
            ? undefined
            : "Enter a positive token count, up to 10,000,000.",
      });
    const contextWindow =
      provider.kind === "echo"
        ? "32768"
        : await integer("Model context window (tokens)", 32768);
    if (!contextWindow) return;
    const output =
      provider.kind === "echo"
        ? "4096"
        : await integer("Maximum output (tokens)", 4096);
    if (!output) return;
    if (
      Number(contextWindow) < 1024 ||
      Number(contextWindow) -
        Number(output) -
        Math.max(512, Math.ceil(Number(contextWindow) / 10)) <=
        0
    )
      throw new UserError(
        "The context window must leave room for input, output, and the runtime safety margin.",
      );
    const profile = `model-${randomUUID().slice(0, 8)}`;
    setup.models.push({
      profile,
      provider_profile: provider.profile,
      model,
      context_window_tokens: Number(contextWindow),
      max_output_tokens: Number(output),
      capabilities: {
        tool_calls: "auto",
        streaming: "auto",
        image_inputs: "auto",
        server_compaction: "auto",
        declared: {},
      },
    });
    setup.primary ||= profile;
  }
  runtimeConfiguration(setup);
  // Recheck trust and membership after native dialogs, before persisting or starting.
  const members = await Promise.all(
    (vscode.workspace.workspaceFolders ?? [])
      .filter((folder) => folder.uri.scheme === "file")
      .map(async (folder) => {
        const current = await workspaceIdentity(folder.uri.fsPath);
        return (
          current.path === selectedWorkspace.path &&
          current.identity === selectedWorkspace.identity
        );
      }),
  );
  if (!vscode.workspace.isTrusted || !members.some(Boolean))
    throw new UserError(
      "The workspace changed during setup. Open the original trusted folder and retry.",
    );
  if (pendingSecret)
    await context.secrets.store(
      credentialKey(pendingSecret.id),
      pendingSecret.value,
    );
  await context.workspaceState.update(managedKey(workspace), setup);
  if (clearedCredential) {
    try {
      await context.secrets.delete(credentialKey(clearedCredential));
    } catch {
      // Retain a reachable reference when OS custody refuses deletion, so the
      // same native action can retry instead of leaving an orphaned key.
      await context.workspaceState.update(managedKey(workspace), saved);
      throw new UserError(
        "The provider key could not be deleted. Retry clearing authentication in Configure Local Runtime.",
      );
    }
  }
  return setup;
}

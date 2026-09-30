import { useState } from "react";
import { IconLoader2 } from "@tabler/icons-react";
import {
  CommandFailure,
  configureSetupCredential,
  codexAuthLogin,
  openSetupLink,
  useSetupModel,
} from "../../api";
import type { DesktopStatus, ManagedCredentialMetadata } from "../../types";
import {
  setupChanged,
  setupProviderReady,
  type SetupPackage,
  type SetupProvider,
} from "../../setupPackages";
import { DropdownSelect } from "../DropdownSelect";
import { MarkdownContent } from "../MarkdownContent";
import { BrowserLinkContext } from "../browser/BrowserLink";

/** Actions for one imported connection, beside its normal inventory details. */
export function ImportedProviderActions({
  item,
  provider,
  desktop,
  credentials,
  busy,
  onChanged,
  showInstructions = true,
}: {
  showInstructions?: boolean;
  item: SetupPackage;
  provider: SetupProvider;
  desktop: DesktopStatus;
  credentials: ManagedCredentialMetadata[];
  busy: boolean;
  onChanged: () => Promise<void>;
}) {
  const [credential, setCredential] = useState("");
  const [model, setModel] = useState(
    provider.models.find((m) => m.profile === item.roles.primary)?.profile ??
      provider.models[0]?.profile ??
      "",
  );
  const [replace, setReplace] = useState(false);
  const [pending, setPending] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const disabled = busy || !!pending;
  async function perform(message: string, action: () => Promise<void>) {
    if (disabled) return;
    setPending(message);
    setError("");
    setNotice("");
    try {
      await action();
      setupChanged();
      await onChanged();
    } catch (failure) {
      setError(
        failure instanceof CommandFailure && failure.detail.violations.length
          ? failure.detail.violations.map((v) => v.description).join(" ")
          : failure instanceof Error
            ? failure.message
            : "Provider setup could not be completed.",
      );
    } finally {
      setPending("");
    }
  }
  return (
    <div
      className="imported-provider-actions"
      aria-label={`Setup for ${provider.displayName}`}
      aria-busy={!!pending}
    >
      <h4>From {item.name}</h4>
      <BrowserLinkContext.Provider
        value={(url, originalHref) => {
          void openSetupLink(item.id, originalHref ?? url).catch(
            (failure: unknown) =>
              setError(
                failure instanceof Error
                  ? failure.message
                  : "Could not open instructions.",
              ),
          );
        }}
      >
        {showInstructions ? (
          <MarkdownContent content={provider.descriptionMarkdown} />
        ) : null}
      </BrowserLinkContext.Provider>
      {provider.credentialRequired ? (
        <div className="imported-provider-control">
          {credentials.length ? (
            <label>
              API key
              <DropdownSelect
                value={credential}
                disabled={disabled}
                onChange={(event) => setCredential(event.target.value)}
              >
                <option value="">Enter a new key</option>
                {credentials.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.label}
                  </option>
                ))}
              </DropdownSelect>
            </label>
          ) : null}
          <button
            type="button"
            className="button secondary"
            disabled={disabled}
            onClick={() =>
              void perform(
                "Waiting for the secure API key window…",
                async () => {
                  await configureSetupCredential({
                    id: item.id,
                    sha256: item.sha256,
                    profile: provider.profile,
                    ...(credential ? { credentialId: credential } : {}),
                  });
                  setNotice(
                    "API key saved. The connection has not been tested.",
                  );
                },
              )
            }
          >
            {credential
              ? "Use saved key"
              : provider.credentialId
                ? "Change API key"
                : "Add API key"}
          </button>
        </div>
      ) : null}
      {provider.kind === "open_ai_codex" &&
      desktop.codexAuth?.state !== "signed_in" ? (
        <button
          type="button"
          className="button secondary"
          disabled={disabled}
          onClick={() =>
            void perform("Complete sign-in in the Codex window…", async () => {
              await codexAuthLogin();
            })
          }
        >
          Sign in to Codex
        </button>
      ) : null}
      {desktop.workspace && provider.models.length ? (
        <>
          <div className="imported-provider-control">
            <label>
              Model for this workspace
              <DropdownSelect
                value={model}
                disabled={disabled}
                onChange={(event) => setModel(event.target.value)}
              >
                {provider.models.map((m) => (
                  <option key={m.profile} value={m.profile}>
                    {m.model}
                  </option>
                ))}
              </DropdownSelect>
            </label>
            <button
              type="button"
              className="button secondary"
              disabled={
                disabled ||
                !setupProviderReady(
                  provider,
                  desktop.codexAuth?.state === "signed_in",
                )
              }
              onClick={() =>
                void perform(
                  "Applying the model… Complete any Desktop confirmation window to continue.",
                  async () => {
                    await useSetupModel({
                      id: item.id,
                      sha256: item.sha256,
                      profile: provider.profile,
                      modelProfile: model,
                      workspaceId: desktop.workspace!.workspaceId,
                      replaceConflicts: replace,
                    });
                    setNotice("Model selected for this workspace.");
                  },
                )
              }
            >
              Use model in workspace
            </button>
          </div>
          <label className="imported-provider-replace">
            <input
              type="checkbox"
              checked={replace}
              disabled={disabled}
              onChange={(event) => setReplace(event.target.checked)}
            />{" "}
            Replace conflicting profiles in this workspace
          </label>
          {!setupProviderReady(
            provider,
            desktop.codexAuth?.state === "signed_in",
          ) ? (
            <p className="setup-hint">
              {provider.kind === "open_ai_codex"
                ? "Sign in to Codex before using this model."
                : "Add an API key before using this model. The provider and models are already saved."}
            </p>
          ) : null}
        </>
      ) : (
        <p className="setup-hint">
          {desktop.workspace
            ? "Add or discover a model to use this connection."
            : "Choose a workspace when you are ready to use these models."}
        </p>
      )}
      {pending ? (
        <p className="setup-action-progress" role="status">
          <IconLoader2
            className="setup-action-spinner"
            size={16}
            aria-hidden="true"
          />
          {pending}
        </p>
      ) : null}
      {error ? (
        <p className="page-error" role="alert">
          {error}
        </p>
      ) : null}
      {notice && !error ? <p role="status">{notice}</p> : null}
    </div>
  );
}

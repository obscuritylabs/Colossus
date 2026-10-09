import { useState } from "react";
import { IconCheck, IconKey } from "@tabler/icons-react";
import { configureSetupCredential, openSetupLink } from "../../api";
import {
  setupChanged,
  setupCredentialStatus,
  type SetupPackage,
  type SetupProvider,
} from "../../setupPackages";
import { ProviderIcon } from "../ProviderIcon";
import { MarkdownContent } from "../MarkdownContent";
import { BrowserLinkContext } from "../browser/BrowserLink";

export interface ImportedProviderSelection {
  package: SetupPackage;
  provider: SetupProvider;
}
export function ImportedProviderPicker({
  packages,
  selected,
  busy,
  signedIn,
  onSelect,
  onOther,
  onBusyChange,
}: {
  packages: SetupPackage[];
  selected: ImportedProviderSelection;
  busy: boolean;
  signedIn: boolean;
  onSelect: (selection: ImportedProviderSelection) => void;
  onOther: () => void;
  onBusyChange: (busy: boolean) => void;
}) {
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const provider = selected.provider;
  async function addKey() {
    onBusyChange(true);
    setError("");
    setNotice("");
    try {
      await configureSetupCredential({
        id: selected.package.id,
        sha256: selected.package.sha256,
        profile: provider.profile,
      });
      setupChanged();
      setNotice("Key saved securely. Connection has not been tested.");
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : "Could not save the API key.",
      );
    } finally {
      onBusyChange(false);
    }
  }
  return (
    <section
      className="imported-provider-picker"
      aria-label="Imported providers"
    >
      <div className="imported-section-heading">
        <div>
          <h2>Your organization’s providers</h2>
          <p>
            Choose one to start this workspace. You can use the others later.
          </p>
        </div>
        <button
          type="button"
          className="text-button"
          disabled={busy}
          onClick={onOther}
        >
          Use another provider
        </button>
      </div>
      <div className="imported-provider-layout">
        <fieldset className="imported-provider-list">
          <legend className="sr-only">Choose an imported provider</legend>
          {packages.flatMap((item) =>
            item.providers.map((entry) => {
              const checked =
                selected.package.id === item.id &&
                provider.profile === entry.profile;
              const primary = entry.models.some(
                (m) => m.profile === item.roles.primary,
              );
              return (
                <label
                  className={`imported-provider-option${checked ? " is-selected" : ""}`}
                  key={`${item.id}:${entry.profile}`}
                >
                  <input
                    type="radio"
                    name="imported-provider"
                    value={`${item.id}:${entry.profile}`}
                    aria-label={entry.displayName}
                    checked={checked}
                    disabled={busy}
                    onChange={() => {
                      setError("");
                      setNotice("");
                      onSelect({ package: item, provider: entry });
                    }}
                  />
                  <ProviderIcon
                    provider={{ kind: entry.kind, baseUrl: entry.baseUrl }}
                    customIcon={entry.icon}
                    customDarkIcon={entry.darkIcon}
                  />
                  <span className="imported-provider-option-copy">
                    <strong>{entry.displayName}</strong>
                    <small>
                      {entry.models.length}{" "}
                      {entry.models.length === 1 ? "model" : "models"} ·{" "}
                      {entry.credentialRequired
                        ? entry.credentialId
                          ? "Key saved"
                          : "Key needed to connect"
                        : entry.kind === "open_ai_codex"
                          ? "ChatGPT account"
                          : "No key needed"}
                    </small>
                  </span>
                  {primary ? (
                    <span className="setup-badge">Recommended</span>
                  ) : null}
                  {checked ? (
                    <IconCheck
                      className="imported-selected-mark"
                      size={17}
                      aria-hidden="true"
                    />
                  ) : null}
                </label>
              );
            }),
          )}
        </fieldset>
        <article
          className="imported-provider-info"
          aria-label={`${provider.displayName} details`}
        >
          <div className="setup-provider-title">
            <ProviderIcon
              provider={{ kind: provider.kind, baseUrl: provider.baseUrl }}
              customIcon={provider.icon}
              customDarkIcon={provider.darkIcon}
            />
            <h3>{provider.displayName}</h3>
          </div>
          <p className="imported-source">From {selected.package.name}</p>
          <dl className="setup-detail-grid">
            <div>
              <dt>Endpoint</dt>
              <dd>
                <code>{provider.baseUrl}</code>
              </dd>
            </div>
            <div>
              <dt>API format</dt>
              <dd>
                {provider.kind === "openai_responses"
                  ? "Responses"
                  : provider.kind === "open_ai_codex"
                    ? "Codex / ChatGPT"
                    : "Chat Completions"}
              </dd>
            </div>
            {provider.timeoutMs ? (
              <div>
                <dt>Connection timeout</dt>
                <dd>{provider.timeoutMs / 1000} seconds</dd>
              </div>
            ) : null}
          </dl>
          <BrowserLinkContext.Provider
            value={(url, originalHref) => {
              void openSetupLink(
                selected.package.id,
                originalHref ?? url,
              ).catch((failure: unknown) =>
                setError(
                  failure instanceof Error
                    ? failure.message
                    : "Could not open link.",
                ),
              );
            }}
          >
            <MarkdownContent content={provider.descriptionMarkdown} />
          </BrowserLinkContext.Provider>
          <div className="imported-key-row">
            <span>
              <IconKey size={16} aria-hidden="true" />
              {setupCredentialStatus(provider, signedIn)}
            </span>
            {provider.credentialRequired ? (
              <button
                type="button"
                className="button secondary"
                disabled={busy}
                onClick={() => void addKey()}
              >
                {provider.credentialId ? "Change API key" : "Add API key"}
              </button>
            ) : null}
          </div>
          {provider.credentialRequired && !provider.credentialId ? (
            <p className="setup-hint">
              Optional for now. You’ll need a key when you start or load models.
            </p>
          ) : null}
          {notice ? (
            <p className="setup-hint" role="status">
              {notice}
            </p>
          ) : null}
          {error ? (
            <p className="page-error" role="alert">
              {error}
            </p>
          ) : null}
        </article>
      </div>
    </section>
  );
}

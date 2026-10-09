import { Fragment, useRef, useState } from "react";
import { useSetupPackages } from "./useSetupPackages";
import { SetupReviewDialog } from "./SetupReviewDialog";
import { SetupGlobalReview } from "./SetupGlobalReview";
import { DropdownSelect } from "../DropdownSelect";
import {
  IconFileImport,
  IconPackage,
  IconChevronDown,
  IconLoader2,
} from "@tabler/icons-react";
import {
  cancelSetupPackageReview,
  inspectSetupPackage,
  applySetupPackage,
  configureSetupCredential,
  useSetupModel,
  exportSetupPackage,
  removeSetupPackage,
  openSetupLink,
  desktopStatus,
  getManagedConfiguration,
  codexAuthLogin,
  CommandFailure,
} from "../../api";
import type { DesktopStatus } from "../../types";
import {
  setupChanged,
  setupContents,
  setupCredentialStatus,
  setupProviderReady,
} from "../../setupPackages";
import type { SetupPackage, SetupProvider } from "../../setupPackages";
import { MarkdownContent } from "../MarkdownContent";
import { BrowserLinkContext } from "../browser/BrowserLink";
import { ProviderIcon } from "../ProviderIcon";
import "./setup-packages.css";

interface Props {
  desktop: DesktopStatus;
  busy?: boolean;
  onStatusChange?:
    ((status: DesktopStatus) => void | Promise<void>) | undefined;
  onSignIn?: () => void | Promise<void>;
  onChooseProvider?: (provider: SetupProvider, packageId: string) => void;
  compact?: boolean;
  inventory?: boolean;
}
export function SetupPackagesPanel({
  desktop,
  busy = false,
  onStatusChange,
  onSignIn,
  onChooseProvider,
  compact = false,
  inventory = false,
}: Props) {
  const { packages } = useSetupPackages();
  const reviewTrigger = useRef<HTMLElement | null>(null);
  const [review, setReview] = useState<SetupPackage | null>(null);
  const [open, setOpen] = useState(false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [trust, setTrust] = useState(false);
  const [replace, setReplace] = useState(false);
  const [applyDefaults, setApplyDefaults] = useState(false);
  const [replaceProfiles, setReplaceProfiles] = useState(false);
  const [credentials, setCredentials] = useState<
    { id: string; label: string }[]
  >([]);
  const [credentialChoices, setCredentialChoices] = useState<
    Record<string, string>
  >({});
  const [modelChoices, setModelChoices] = useState<Record<string, string>>({});
  const disabled = busy || working;
  const signedIn = desktop.codexAuth?.state === "signed_in";

  async function refresh() {
    setupChanged();
    const snapshot = await getManagedConfiguration();
    setCredentials(snapshot.globalConfiguration.credentials);
  }
  async function perform(action: () => Promise<void>) {
    if (disabled) return;
    setWorking(true);
    setError("");
    setNotice("");
    try {
      await action();
    } catch (failure) {
      setError(
        failure instanceof CommandFailure && failure.detail.violations.length
          ? failure.detail.violations
              .map((violation) => violation.description)
              .join(" ")
          : failure instanceof Error
            ? failure.message
            : "Setup could not be completed.",
      );
    } finally {
      setWorking(false);
    }
  }
  async function inspect(id: string | null = null) {
    reviewTrigger.current = document.activeElement as HTMLElement | null;
    await perform(async () => {
      const proposal = await inspectSetupPackage(id);
      if (!proposal) return;
      setReview(proposal);
      setTrust(false);
      setReplace(false);
      setApplyDefaults(false);
      setOpen(true);
    });
  }
  async function cancelReview() {
    if (!review) return;
    await perform(async () => {
      await cancelSetupPackageReview(review.sha256);
      setReview(null);
    });
  }
  async function apply() {
    if (!review) return;
    await perform(async () => {
      await applySetupPackage({
        sha256: review.sha256,
        trustCertificates: trust,
        replaceExisting: replace,
        applyDefaults,
      });
      setReview(null);
      await refresh();
      setupChanged();
      await onStatusChange?.(trust ? await desktopStatus() : desktop);
      setOpen(false);
      setNotice(
        "Setup imported. Your entries are available in settings; add credentials whenever you are ready.",
      );
    });
  }
  function instructions(content: string, id: string) {
    return (
      <BrowserLinkContext.Provider
        value={(url, originalHref) => {
          void perform(() => openSetupLink(id, originalHref ?? url));
        }}
      >
        <MarkdownContent content={content} />
      </BrowserLinkContext.Provider>
    );
  }
  function providerDetails(provider: SetupProvider, item: SetupPackage) {
    return (
      <div className="setup-provider-details">
        <dl className="setup-detail-grid">
          <div>
            <dt>API endpoint</dt>
            <dd>
              <code>{provider.baseUrl}</code>
            </dd>
          </div>
          <div>
            <dt>API format</dt>
            <dd>
              {provider.kind === "open_ai_codex"
                ? "Codex / ChatGPT"
                : provider.kind === "openai_responses"
                  ? "Responses"
                  : "Chat Completions"}
            </dd>
          </div>
          <div>
            <dt>Connection timeout</dt>
            <dd>
              {provider.timeoutMs
                ? `${provider.timeoutMs / 1000} seconds`
                : "Automatic"}
            </dd>
          </div>
          <div>
            <dt>Profile ID</dt>
            <dd>
              <code>{provider.profile}</code>
            </dd>
          </div>
        </dl>
        {instructions(provider.descriptionMarkdown, item.id)}
        {provider.models.length ? (
          <div className="setup-table-scroll">
            <table className="setup-model-table">
              <caption>Included model settings</caption>
              <thead>
                <tr>
                  <th>Model</th>
                  <th>Context / output tokens</th>
                  <th>Capabilities</th>
                  <th>Suggested roles</th>
                </tr>
              </thead>
              <tbody>
                {provider.models.map((model) => (
                  <tr key={model.profile}>
                    <td>
                      <strong>{model.profile}</strong>
                      <br />
                      <code>{model.model}</code>
                    </td>
                    <td>
                      {model.contextWindowTokens.toLocaleString()} /{" "}
                      {model.maxOutputTokens.toLocaleString()}
                    </td>
                    <td>
                      {[
                        model.capabilities.toolCalls !== "off" && "Tools",
                        model.capabilities.streaming !== "off" && "Streaming",
                        model.capabilities.imageInputs !== "off" && "Images",
                      ]
                        .filter(Boolean)
                        .join(", ") || "Text"}
                      {model.reasoningEffort
                        ? ` · ${model.reasoningEffort} reasoning`
                        : ""}
                    </td>
                    <td>
                      {Object.entries(item.roles)
                        .filter(([, target]) => target === model.profile)
                        .map(([role]) => role)
                        .join(", ") || "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <p>
            Choose or discover models after selecting a workspace. Importing
            does not contact this provider.
          </p>
        )}
      </div>
    );
  }
  function providerTable(item: SetupPackage, preview: boolean) {
    return (
      <div className="setup-table-scroll">
        <table className="setup-provider-table">
          <caption>
            {preview ? "Providers in this setup file" : "Imported providers"}
          </caption>
          <thead>
            <tr>
              <th>Provider and endpoint</th>
              <th>Models</th>
              <th>Credentials</th>
              <th>{preview ? "Suggested model" : "Setup actions"}</th>
            </tr>
          </thead>
          <tbody>
            {item.providers.map((provider) => {
              const key = `${item.id}:${provider.profile}`;
              const model =
                modelChoices[key] ??
                provider.models.find((m) => item.roles.primary === m.profile)
                  ?.profile ??
                provider.models[0]?.profile ??
                "";
              return (
                <Fragment key={provider.profile}>
                  <tr>
                    <td>
                      <div className="setup-provider-title">
                        <ProviderIcon
                          provider={{
                            kind: provider.kind,
                            baseUrl: provider.baseUrl,
                          }}
                          customIcon={provider.icon}
                          customDarkIcon={provider.darkIcon}
                        />
                        <strong>{provider.displayName}</strong>
                      </div>
                      <code className="setup-endpoint">{provider.baseUrl}</code>
                    </td>
                    <td>{provider.models.length || "Choose later"}</td>
                    <td>
                      <span
                        className={
                          setupProviderReady(provider, signedIn)
                            ? "setup-status-ready"
                            : "setup-status-pending"
                        }
                      >
                        {setupCredentialStatus(provider, signedIn)}
                      </span>
                    </td>
                    <td>
                      {preview ? (
                        <code className="setup-endpoint">
                          {provider.models.find(
                            (entry) => entry.profile === item.roles.primary,
                          )?.model ??
                            provider.models[0]?.model ??
                            "Choose later"}
                        </code>
                      ) : (
                        <div className="setup-row-actions">
                          {provider.credentialRequired ? (
                            <>
                              {credentials.length ? (
                                <DropdownSelect
                                  aria-label={`Saved credential for ${provider.displayName}`}
                                  value={credentialChoices[key] ?? ""}
                                  disabled={disabled}
                                  onChange={(event) =>
                                    setCredentialChoices({
                                      ...credentialChoices,
                                      [key]: event.target.value,
                                    })
                                  }
                                >
                                  <option value="">Enter a new key</option>
                                  {credentials.map((credential) => (
                                    <option
                                      key={credential.id}
                                      value={credential.id}
                                    >
                                      {credential.label}
                                    </option>
                                  ))}
                                </DropdownSelect>
                              ) : null}
                              <button
                                type="button"
                                className="button secondary"
                                disabled={disabled}
                                onClick={() =>
                                  void perform(async () => {
                                    await configureSetupCredential({
                                      id: item.id,
                                      sha256: item.sha256,
                                      profile: provider.profile,
                                      ...(credentialChoices[key]
                                        ? {
                                            credentialId:
                                              credentialChoices[key],
                                          }
                                        : {}),
                                    });
                                    await refresh();
                                    setupChanged();
                                    setNotice(
                                      "API key saved. Connection has not been tested.",
                                    );
                                  })
                                }
                              >
                                {credentialChoices[key]
                                  ? "Use saved key"
                                  : provider.credentialId
                                    ? "Change API key"
                                    : "Add API key"}
                              </button>
                            </>
                          ) : provider.kind === "open_ai_codex" && !signedIn ? (
                            <button
                              type="button"
                              className="button secondary"
                              disabled={disabled}
                              onClick={() =>
                                void perform(async () => {
                                  if (onSignIn) await onSignIn();
                                  else {
                                    const codexAuth = await codexAuthLogin();
                                    await onStatusChange?.({
                                      ...desktop,
                                      codexAuth,
                                    });
                                  }
                                })
                              }
                            >
                              Sign in
                            </button>
                          ) : null}
                          {desktop.workspace && provider.models.length ? (
                            <>
                              <DropdownSelect
                                aria-label={`Model for ${provider.displayName}`}
                                value={model}
                                disabled={disabled}
                                onChange={(event) =>
                                  setModelChoices({
                                    ...modelChoices,
                                    [key]: event.target.value,
                                  })
                                }
                              >
                                {provider.models.map((m) => (
                                  <option key={m.profile} value={m.profile}>
                                    {m.profile} · {m.model}
                                  </option>
                                ))}
                              </DropdownSelect>
                              <button
                                className="button primary"
                                type="button"
                                disabled={
                                  disabled ||
                                  !setupProviderReady(provider, signedIn)
                                }
                                onClick={() =>
                                  void perform(async () => {
                                    const status = await useSetupModel({
                                      id: item.id,
                                      sha256: item.sha256,
                                      profile: provider.profile,
                                      modelProfile: model,
                                      workspaceId:
                                        desktop.workspace!.workspaceId,
                                      replaceConflicts: replaceProfiles,
                                    });
                                    await onStatusChange?.(status);
                                    setupChanged();
                                    setNotice(
                                      "Model selected for this workspace.",
                                    );
                                  })
                                }
                              >
                                Use in workspace
                              </button>
                            </>
                          ) : desktop.workspace ? (
                            <button
                              className="button secondary"
                              type="button"
                              disabled={disabled || !onChooseProvider}
                              onClick={() =>
                                onChooseProvider?.(provider, item.id)
                              }
                            >
                              Choose models
                            </button>
                          ) : (
                            <span>Choose a workspace to use these models.</span>
                          )}
                        </div>
                      )}
                    </td>
                  </tr>
                  <tr className="setup-provider-detail-row">
                    <td colSpan={4}>
                      <details>
                        <summary>
                          View instructions and configuration{" "}
                          <IconChevronDown size={14} />
                        </summary>
                        {providerDetails(provider, item)}
                      </details>
                    </td>
                  </tr>
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>
    );
  }
  function installedPackages() {
    return (
      <div className="setup-installed-packages">
        {!inventory && desktop.provider.configured ? (
          <label className="setup-replace">
            <input
              type="checkbox"
              checked={replaceProfiles}
              disabled={disabled}
              onChange={(event) => setReplaceProfiles(event.target.checked)}
            />{" "}
            Replace matching provider and model profiles when selecting a model
            for this workspace
          </label>
        ) : null}
        {packages.map((item) => (
          <article key={item.id}>
            <h2>
              {item.name} <span>v{item.version}</span>
            </h2>
            {inventory ? (
              <p>{setupContents(item)}</p>
            ) : (
              <>
                {instructions(item.descriptionMarkdown, item.id)}
                {providerTable(item, false)}
              </>
            )}
            <div className="setup-package-footer">
              {
                <button
                  type="button"
                  className="text-button"
                  disabled={disabled}
                  onClick={() => void inspect(item.id)}
                >
                  Review setup
                </button>
              }
              <button
                type="button"
                className="text-button"
                disabled={disabled}
                onClick={() =>
                  void perform(async () => {
                    if (await exportSetupPackage(item.id))
                      setNotice("Setup file exported without credentials.");
                  })
                }
              >
                Export setup file
              </button>
              <details>
                <summary>Remove saved setup</summary>
                <p>
                  Removes this saved setup file and its instructions. Providers,
                  models, workspaces, credentials, and trusted certificates are
                  retained.
                </p>
                <button
                  className="button secondary"
                  type="button"
                  disabled={disabled}
                  onClick={() =>
                    void perform(async () => {
                      await removeSetupPackage({
                        id: item.id,
                        sha256: item.sha256,
                      });
                      await refresh();
                      setupChanged();
                    })
                  }
                >
                  Remove {item.name}
                </button>
              </details>
            </div>
          </article>
        ))}
      </div>
    );
  }
  return (
    <section
      className={`setup-packages${open && !compact && !inventory ? "" : " setup-packages--collapsed"}`}
      aria-label="Desktop setup files"
      aria-busy={working}
    >
      <div className="setup-package-toolbar">
        <div>
          <strong className="setup-package-title">
            <IconPackage size={18} /> Desktop setup file
          </strong>
          <p>
            Share providers, models, global defaults, MCP, search, telemetry,
            and certificates.
          </p>
        </div>
        <button
          className="button secondary"
          type="button"
          disabled={disabled}
          onClick={() => void inspect()}
        >
          <IconFileImport size={17} /> Import setup file
        </button>
        {packages.length && (!compact || inventory) ? (
          <button
            className="text-button"
            type="button"
            disabled={disabled}
            onClick={(event) => {
              reviewTrigger.current = event.currentTarget;
              setOpen(!open);
              if (!open) void perform(refresh);
            }}
          >
            {inventory
              ? `Manage setup files (${packages.length})`
              : open
                ? "Hide imported providers"
                : `View imported providers (${packages.reduce((n, p) => n + p.providers.length, 0)})`}
          </button>
        ) : null}
        <button
          className="text-button"
          type="button"
          disabled={disabled}
          onClick={() =>
            void perform(async () => {
              if (await exportSetupPackage(null))
                setNotice(
                  "Global setup exported. Stored secrets and workspace data are excluded.",
                );
            })
          }
        >
          Export global setup
        </button>
      </div>
      {error && !review && !(inventory && open) ? (
        <p className="page-error" role="alert">
          {error}
        </p>
      ) : null}
      {notice ? <p role="status">{notice}</p> : null}
      {compact && !inventory && packages.length ? (
        <div className="setup-import-summary">
          {packages.map((item) => (
            <div key={item.id}>
              <strong>{item.name}</strong>
              <span>File includes {setupContents(item)}</span>
              {
                <button
                  type="button"
                  className="text-button"
                  disabled={disabled}
                  onClick={() => void inspect(item.id)}
                >
                  Review setup
                </button>
              }
            </div>
          ))}
        </div>
      ) : null}
      {review ? (
        <SetupReviewDialog
          busy={disabled}
          returnFocus={reviewTrigger.current}
          onClose={() => void cancelReview()}
        >
          <div className="setup-package-review">
            <h2 id="setup-review-title">Review {review.name}</h2>
            <p>
              Version {review.version} · {setupContents(review)}
            </p>
            {instructions(review.descriptionMarkdown, review.id)}
            {review.certificateFingerprints.length ? (
              <fieldset disabled={disabled} className="setup-trust-review">
                <legend>Included CA certificates</legend>
                <p>
                  Trust applies across Colossus-owned connections on this
                  computer. Operating system trust is unchanged.
                </p>
                <details>
                  <summary>
                    View {review.certificateFingerprints.length} certificate
                    fingerprints
                  </summary>
                  <ul>
                    {review.certificateFingerprints.map((fingerprint) => (
                      <li key={fingerprint}>
                        <code>{fingerprint}</code>
                      </li>
                    ))}
                  </ul>
                </details>
                {review.existingCertificateFingerprints.length ? (
                  <p>
                    This will replace the{" "}
                    {review.existingCertificateFingerprints.length} certificates
                    in your current additional CA bundle.
                  </p>
                ) : null}
                <label>
                  <input
                    type="checkbox"
                    checked={trust}
                    onChange={(event) => setTrust(event.target.checked)}
                  />{" "}
                  Trust the included CA certificates in Colossus
                </label>
              </fieldset>
            ) : (
              <p>
                No CA certificates are included. Existing trust settings stay in
                place.
              </p>
            )}
            {review.providers.length ? providerTable(review, true) : null}
            <SetupGlobalReview
              settings={review.globalSettings}
              applyDefaults={applyDefaults}
              onApplyDefaults={setApplyDefaults}
              disabled={disabled}
            />
            {review.replacesVersion ? (
              <label className="setup-replace">
                <input
                  type="checkbox"
                  checked={replace}
                  disabled={disabled}
                  onChange={(event) => setReplace(event.target.checked)}
                />{" "}
                Replace saved setup “{review.name}” version{" "}
                {review.replacesVersion}. Existing workspace configurations keep
                their current settings.
              </label>
            ) : null}
            {error ? (
              <p className="page-error" role="alert">
                {error}
              </p>
            ) : null}
            <p>
              Credentials are optional now. You can complete connection setup
              later.
            </p>
            <div className="setup-package-footer">
              <button
                className="button secondary"
                type="button"
                disabled={disabled}
                onClick={() => void cancelReview()}
              >
                Cancel
              </button>
              <button
                className="button primary"
                type="button"
                disabled={disabled || (!!review.replacesVersion && !replace)}
                onClick={() => void apply()}
              >
                Import setup
              </button>
            </div>
          </div>
        </SetupReviewDialog>
      ) : open && (!compact || inventory) ? (
        inventory ? (
          <SetupReviewDialog
            busy={disabled}
            returnFocus={reviewTrigger.current}
            onClose={() => setOpen(false)}
          >
            <h2 id="setup-review-title">Setup files</h2>
            <p>
              Imported providers and models are in your inventory. Expand a
              provider to add a key or choose a model for your workspace.
            </p>
            {error ? (
              <p role="alert" className="page-error">
                {error}
              </p>
            ) : null}
            {working ? (
              <p role="status">
                <IconLoader2 className="setup-action-spinner" size={16} />{" "}
                Updating setup…
              </p>
            ) : null}
            {installedPackages()}
            <div className="setup-package-footer">
              <button
                className="button secondary"
                disabled={disabled}
                onClick={() => setOpen(false)}
              >
                Done
              </button>
            </div>
          </SetupReviewDialog>
        ) : (
          installedPackages()
        )
      ) : null}
    </section>
  );
}

import { DropdownSelect } from "./DropdownSelect";
import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import type { PluginEntry, PluginRequest, PluginSource } from "../plugins";

export type PluginAction =
  | "add"
  | "workspace_accept"
  | "workspace_disable"
  | "install"
  | "validate"
  | "verify"
  | "package"
  | "pull"
  | "push"
  | "gc"
  | "enable"
  | "disable"
  | "update"
  | "uninstall"
  | "export"
  | "verify_installed";

export function PluginOperationForm({
  action,
  plugin,
  busy,
  onSubmit,
  onClose,
}: {
  action: PluginAction;
  plugin?: PluginEntry;
  busy: boolean;
  onSubmit: (request: PluginRequest, verifyArchive: boolean) => void;
  onClose: () => void;
}) {
  const formRef = useRef<HTMLFormElement>(null);
  useEffect(() => {
    const trigger = document.activeElement;
    formRef.current?.focus();
    return () => {
      if (trigger instanceof HTMLElement && trigger.isConnected)
        trigger.focus({ preventScroll: true });
    };
  }, []);
  const [source, setSource] = useState<PluginSource["kind"]>("reference");
  const [digest, setDigest] = useState("");
  const [registry, setRegistry] = useState("");
  const [reference, setReference] = useState("");
  const [profile, setProfile] = useState("default");
  const [archive, setArchive] = useState(false);
  const [purge, setPurge] = useState(false);
  const [untrusted, setUntrusted] = useState(false);
  const label =
    action === "workspace_accept"
      ? "Use workspace source"
      : action === "workspace_disable"
        ? "Disable workspace source"
        : action.replaceAll("_", " ");
  const network =
    ["pull", "push", "update"].includes(action) ||
    (["add", "install"].includes(action) && source === "reference");
  function submit(event: FormEvent) {
    event.preventDefault();
    let request: PluginRequest;
    const name = plugin?.manifest.name ?? "";
    const identity = plugin?.digest ?? "";
    switch (action) {
      case "add":
      case "install": {
        const input: PluginSource =
          source === "reference"
            ? {
                kind: source,
                registry: registry.trim(),
                reference: reference.trim().replace(/^oci:\/\//, ""),
              }
            : source === "directory"
              ? { kind: source, path: "" }
              : { kind: source, path: "", digest: digest || null };
        request = { operation: action, source: input, trust_profile: profile };
        break;
      }
      case "workspace_accept":
        request = {
          operation: "accept_workspace",
          path: plugin?.source ?? "",
          digest: identity,
        };
        break;
      case "workspace_disable":
        request = {
          operation: "disable_workspace",
          path: plugin?.source ?? "",
        };
        break;
      case "verify":
        request = {
          operation: action,
          path: "",
          digest: digest || null,
          trust_profile: source === "reference" ? "default" : profile,
        };
        break;
      case "validate":
        request = { operation: action, path: "" };
        break;
      case "package":
        request = { operation: action, directory: "", output: "" };
        break;
      case "pull":
        request = { operation: action, registry, reference, output: "" };
        break;
      case "push":
        request = { operation: action, registry, reference, layout: "" };
        break;
      case "update":
        request = { operation: action, name, registry, reference };
        break;
      case "enable":
        request = {
          operation: action,
          name,
          digest: identity,
          allow_untrusted: untrusted,
        };
        break;
      case "disable":
        request = { operation: action, name };
        break;
      case "uninstall":
        request = {
          operation: action,
          name,
          digest: identity,
          purge_data: purge,
        };
        break;
      case "export":
        request = { operation: action, name, output: "" };
        break;
      case "verify_installed":
        request = { operation: action, name, digest: identity };
        break;
      case "gc":
        request = { operation: action };
        break;
    }
    onSubmit(request, archive);
  }
  return (
    <form
      ref={formRef}
      tabIndex={-1}
      className="plugin-operation"
      onSubmit={submit}
      aria-label={`${label} plugin`}
    >
      <h3>
        {label}
        {plugin ? ` · ${plugin.manifest.name}` : ""}
      </h3>
      <p>
        {action === "workspace_accept" ||
        action === "workspace_disable" ||
        (action === "add" && source === "directory")
          ? "This source is used only in this workspace. Acceptance also covers later instruction edits in this directory. Tools and connections require separate permission."
          : "Installed plugins are shared by workspaces using this Colossus home. Workspace exclusions remain in Settings."}
      </p>
      <fieldset disabled={busy}>
        {["add", "install"].includes(action) && (
          <label>
            Plugin source
            <DropdownSelect
              value={source}
              onChange={(event) =>
                setSource(event.target.value as PluginSource["kind"])
              }
            >
              <option value="reference">OCI registry</option>
              <option value="directory">Plugin directory</option>
              <option value="layout">OCI layout directory</option>
              <option value="archive">OCI layout archive</option>
            </DropdownSelect>
          </label>
        )}
        {network && (
          <>
            <label>
              {["add", "install"].includes(action)
                ? "OCI plugin reference"
                : "Registry reference"}
              <input
                required
                value={reference}
                onChange={(event) => setReference(event.target.value)}
                placeholder="oci://ghcr.io/obscuritylabs/colossus-plugin-outlook-classic:version"
              />
            </label>
            {["add", "install"].includes(action) ? (
              <details>
                <summary>Registry options</summary>
                <label>
                  Registry profile
                  <input
                    value={registry}
                    onChange={(event) => setRegistry(event.target.value)}
                    placeholder="Selected automatically when one profile matches"
                  />
                </label>
              </details>
            ) : (
              <label>
                Registry profile
                <input
                  required
                  value={registry}
                  onChange={(event) => setRegistry(event.target.value)}
                  placeholder="Configured registry name"
                />
              </label>
            )}
          </>
        )}
        {((action === "install" && source !== "reference") ||
          (action === "add" && ["layout", "archive"].includes(source)) ||
          action === "verify") && (
          <label>
            Trust profile
            <input
              required
              value={profile}
              onChange={(event) => setProfile(event.target.value)}
            />
          </label>
        )}
        {["add", "install"].includes(action) && source === "reference" && (
          <p>
            The matching registry profile verifies the package signature. The
            built-in Obscurity Labs profile accepts only its published plugin
            workflow identity.
          </p>
        )}
        {(action === "verify" ||
          (["add", "install"].includes(action) &&
            (source === "layout" || source === "archive"))) && (
          <label>
            Exact manifest digest
            <input
              value={digest}
              pattern="sha256:[0-9a-f]{64}"
              onChange={(event) => setDigest(event.target.value)}
              placeholder="Required when a layout has multiple candidates"
            />
          </label>
        )}
        {action === "verify" && (
          <label>
            <input
              type="checkbox"
              checked={archive}
              onChange={(event) => setArchive(event.target.checked)}
            />{" "}
            Select a layout archive instead of a directory
          </label>
        )}
        {action === "enable" &&
          plugin?.origin !== "bundled" &&
          !plugin?.trust.trusted && (
            <label>
              <input
                type="checkbox"
                checked={untrusted}
                onChange={(event) => setUntrusted(event.target.checked)}
              />{" "}
              Request approval to enable untrusted content. This checkbox does
              not authorize it.
            </label>
          )}
        {action === "uninstall" && (
          <label>
            <input
              type="checkbox"
              checked={purge}
              onChange={(event) => setPurge(event.target.checked)}
            />{" "}
            Also permanently remove this plugin’s writable data
          </label>
        )}
        {plugin &&
          (action === "workspace_accept" || action === "workspace_disable") && (
            <p>
              Directory: <code>{plugin.source}</code>
            </p>
          )}
        {plugin && (
          <p className="plugin-digest">Exact version: {plugin.digest}</p>
        )}
        {action === "install" || action === "update" ? (
          <p>
            The candidate is installed disabled. Activate its exact digest
            separately.
          </p>
        ) : null}
        {action === "add" && source !== "directory" && (
          <p>
            The signature is verified before the exact installed version is
            activated.
          </p>
        )}
        {[
          "add",
          "install",
          "validate",
          "verify",
          "package",
          "pull",
          "push",
          "export",
        ].includes(action) && (
          <p>
            Native file dialogs select paths for this Managed Local target.
            Existing output paths are never silently overwritten.
          </p>
        )}
        <div className="plugin-actions">
          <button type="submit" className="button primary">
            {action === "workspace_accept" || action === "workspace_disable"
              ? "Continue"
              : `Continue ${label}`}
          </button>
          <button type="button" className="button secondary" onClick={onClose}>
            Close
          </button>
        </div>
      </fieldset>
    </form>
  );
}

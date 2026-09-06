import { DropdownSelect } from "./DropdownSelect";
import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import type { PluginEntry, PluginRequest, PluginSource } from "../plugins";

export type PluginAction =
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
  const [source, setSource] = useState<PluginSource["kind"]>("directory");
  const [digest, setDigest] = useState("");
  const [registry, setRegistry] = useState("");
  const [reference, setReference] = useState("");
  const [profile, setProfile] = useState("default");
  const [archive, setArchive] = useState(false);
  const [purge, setPurge] = useState(false);
  const [untrusted, setUntrusted] = useState(false);
  const network =
    ["pull", "push", "update"].includes(action) ||
    (action === "install" && source === "reference");
  function submit(event: FormEvent) {
    event.preventDefault();
    let request: PluginRequest;
    const name = plugin?.manifest.name ?? "";
    const identity = plugin?.digest ?? "";
    switch (action) {
      case "install": {
        const input: PluginSource =
          source === "reference"
            ? { kind: source, registry, reference }
            : source === "directory"
              ? { kind: source, path: "" }
              : { kind: source, path: "", digest: digest || null };
        request = { operation: action, source: input, trust_profile: profile };
        break;
      }
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
      aria-label={`${action.replaceAll("_", " ")} plugin`}
    >
      <h3>
        {action.replaceAll("_", " ")}
        {plugin ? ` · ${plugin.manifest.name}` : ""}
      </h3>
      <p>
        Lifecycle changes affect every workspace sharing this Colossus home.
        Workspace exclusions remain in Settings.
      </p>
      <fieldset disabled={busy}>
        {action === "install" && (
          <label>
            Installation source
            <DropdownSelect
              value={source}
              onChange={(event) =>
                setSource(event.target.value as PluginSource["kind"])
              }
            >
              <option value="directory">Plugin directory</option>
              <option value="layout">OCI layout directory</option>
              <option value="archive">OCI layout archive</option>
              <option value="reference">Registry reference</option>
            </DropdownSelect>
          </label>
        )}
        {network && (
          <>
            <label>
              Registry profile
              <input
                required
                value={registry}
                onChange={(event) => setRegistry(event.target.value)}
                placeholder="Configured registry name"
              />
            </label>
            <label>
              Registry reference
              <input
                required
                value={reference}
                onChange={(event) => setReference(event.target.value)}
                placeholder="registry.example.com/team/plugin:version"
              />
            </label>
          </>
        )}
        {((action === "install" && source !== "reference") ||
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
        {action === "install" && source === "reference" && (
          <p>
            Registry installations enforce the trust profile configured for this
            registry.
          </p>
        )}
        {(action === "verify" ||
          (action === "install" &&
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
        {plugin && (
          <p className="plugin-digest">Exact version: {plugin.digest}</p>
        )}
        {action === "install" || action === "update" ? (
          <p>
            The candidate is installed disabled. Activate its exact digest
            separately.
          </p>
        ) : null}
        {[
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
            Continue {action.replaceAll("_", " ")}
          </button>
          <button type="button" className="button secondary" onClick={onClose}>
            Close
          </button>
        </div>
      </fieldset>
    </form>
  );
}

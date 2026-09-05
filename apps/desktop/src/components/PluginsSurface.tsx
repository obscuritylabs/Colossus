import { useCallback, useEffect, useRef, useState } from "react";
import {
  cancelPluginOperation,
  getPluginInventory,
  managePlugin,
  readPluginPreview,
} from "../api";
import { filterPlugins } from "../plugins";
import type {
  PluginEntry,
  PluginInventory,
  PluginRequest,
  PluginResource,
  PluginSkill,
} from "../plugins";
import { PluginOperationForm } from "./PluginOperationForm";
import { PluginMcpControls } from "./PluginMcpControls";
import type { PluginAction } from "./PluginOperationForm";
import "./plugins.css";

function failure(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "The plugin request failed. Check the selected target and retry.";
}

export function PluginsSurface({
  targetId,
  supported,
  selections = [],
  onUseSkill,
}: {
  targetId: string | null;
  supported: boolean;
  selections?: readonly string[] | undefined;
  onUseSkill?: ((id: string) => void) | undefined;
}) {
  // The parent keys this view by target. Results from an old target cannot populate a new one.
  const [inventory, setInventory] = useState<PluginInventory | null>(null);
  const [error, setError] = useState("");
  const [inventoryError, setInventoryError] = useState("");
  const [message, setMessage] = useState("");
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [operation, setOperation] = useState<{
    action: PluginAction;
    plugin?: PluginEntry;
  } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const refresh = useCallback(async () => {
    if (!targetId || !supported || inFlight.current) return;
    inFlight.current = true;
    setLoading(true);
    try {
      const next = await getPluginInventory(targetId);
      if (mounted.current) {
        setInventory(next);
        setInventoryError("");
      }
    } catch (error) {
      if (mounted.current) setInventoryError(failure(error));
    } finally {
      inFlight.current = false;
      if (mounted.current) setLoading(false);
    }
  }, [targetId, supported]);
  useEffect(() => {
    mounted.current = true;
    void refresh();
    const visibleRefresh = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    window.addEventListener("focus", visibleRefresh);
    const timer = window.setInterval(visibleRefresh, 15_000);
    return () => {
      mounted.current = false;
      window.removeEventListener("focus", visibleRefresh);
      window.clearInterval(timer);
    };
  }, [refresh]);
  async function perform(request: PluginRequest, archive: boolean) {
    if (!targetId || busy) return;
    const id = crypto.randomUUID();
    setBusy(id);
    setError("");
    setMessage(
      "Waiting for the runtime. File selection or native policy approval may be required.",
    );
    try {
      const result = await managePlugin(targetId, id, request, archive);
      if (mounted.current) {
        setMessage(
          result.cancelled
            ? "Operation cancelled."
            : `${request.operation.replaceAll("_", " ")} completed.${result.integrity ? ` Integrity: ${result.integrity}.` : ""}${request.operation === "install" || request.operation === "update" ? " Candidate installed; activation is separate." : ""}`,
        );
        setOperation(null);
      }
    } catch (error) {
      if (mounted.current) {
        setError(failure(error));
        setMessage("");
      }
    } finally {
      if (mounted.current) {
        setBusy(null);
        await refresh();
      }
    }
  }
  if (!targetId || !supported)
    return (
      <section className="overview-scroll plugin-surface">
        <h2>Plugins</h2>
        <p role="status">
          This target does not advertise authorized plugin discovery. Select a
          supported target or update its runtime.
        </p>
      </section>
    );
  const plugin = inventory?.plugins.find((entry) => entry.digest === selected);
  return (
    <section className="overview-scroll plugin-surface" aria-label="Plugins">
      <header>
        <h2>Plugins</h2>
        <p>
          Skills ship with Colossus and installed Agent Plugins. Browsing never
          activates skills or MCP servers.
        </p>
      </header>
      <div className="plugin-actions">
        <label>
          Search plugins and skills
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <button
          className="button secondary"
          disabled={loading}
          onClick={() => void refresh()}
        >
          Refresh plugins
        </button>
      </div>
      {inventory?.managementAvailable ? (
        <div className="plugin-actions" aria-label="Plugin management">
          {(
            [
              "install",
              "validate",
              "verify",
              "package",
              "pull",
              "push",
              "gc",
            ] as const
          ).map((action) => (
            <button
              className="button secondary"
              disabled={busy !== null}
              key={action}
              onClick={() => setOperation({ action })}
            >
              {action === "gc"
                ? "Garbage collect"
                : action[0]!.toUpperCase() + action.slice(1)}
            </button>
          ))}
        </div>
      ) : (
        <p>
          Read-only discovery. Lifecycle management is available on Managed
          Local.
        </p>
      )}
      {loading && <p role="status">Refreshing plugin inventory…</p>}
      {error && <p role="alert">{error}</p>}
      {inventoryError && <p role="alert">{inventoryError}</p>}
      {message && <p role="status">{message}</p>}
      {busy && (
        <button
          className="button secondary"
          onClick={() => {
            void cancelPluginOperation(targetId, busy)
              .then(() => {
                if (mounted.current)
                  setMessage(
                    "Cancellation requested. A committed change is not rolled back; refresh before retrying.",
                  );
              })
              .catch((error: unknown) => {
                if (mounted.current) setError(failure(error));
              });
          }}
        >
          Cancel operation
        </button>
      )}
      {operation && (
        <PluginOperationForm
          key={`${operation.action}:${operation.plugin?.digest ?? "new"}`}
          {...operation}
          busy={busy !== null}
          onSubmit={(request, archive) => void perform(request, archive)}
          onClose={() => setOperation(null)}
        />
      )}
      {inventory && filterPlugins(inventory.plugins, query).length === 0 && (
        <p>No plugins match this view.</p>
      )}
      <div className="plugin-layout">
        <div className="plugin-inventory">
          {filterPlugins(inventory?.plugins ?? [], query).map((entry) => (
            <button
              type="button"
              className="plugin-card"
              key={entry.digest}
              aria-pressed={selected === entry.digest}
              onClick={() => setSelected(entry.digest)}
            >
              <strong>{entry.manifest.name}</strong>
              <span>{entry.manifest.version ?? "Unversioned"}</span>
              <span>
                {entry.origin === "bundled"
                  ? "Bundled with Colossus"
                  : entry.source}
              </span>
              <span>
                Global: {entry.status} · Workspace:{" "}
                {entry.available ? "available" : "unavailable"}
              </span>
              <small>{entry.manifest.description}</small>
            </button>
          ))}
        </div>
        {plugin ? (
          <article
            className="plugin-detail"
            aria-label={`${plugin.manifest.name} details`}
          >
            <h3>{plugin.manifest.name}</h3>
            <p>{plugin.manifest.description}</p>
            <p className="plugin-digest">{plugin.digest}</p>
            <p>
              {plugin.origin === "bundled"
                ? "Bundled with Colossus. Version managed by the executable; this is not a Cosign signature."
                : `${plugin.trust.trusted ? "Signature verified" : "Untrusted"} · ${plugin.trust.method} · ${plugin.trust.profile ?? "No trust profile"}`}
            </p>
            {plugin.trust.signer && <p>Signer: {plugin.trust.signer}</p>}
            {!plugin.available && (
              <p role="status">
                {plugin.unavailable_reason ?? "Unavailable in this workspace"}
              </p>
            )}
            {inventory?.managementAvailable && (
              <div className="plugin-actions">
                {plugin.actions
                  .filter(
                    (action) =>
                      action !== "inspect" &&
                      (action !== "enable" || plugin.status !== "enabled") &&
                      (action !== "disable" || plugin.status === "enabled"),
                  )
                  .map((action) => (
                    <button
                      className="button secondary"
                      key={action}
                      disabled={busy !== null}
                      onClick={() =>
                        setOperation({
                          action: (action === "verify"
                            ? "verify_installed"
                            : action) as PluginAction,
                          plugin,
                        })
                      }
                    >
                      {action === "enable"
                        ? "Activate this digest"
                        : action[0]!.toUpperCase() + action.slice(1)}
                    </button>
                  ))}
              </div>
            )}
            <h4>Skills</h4>
            {plugin.skills.length === 0 && (
              <p>No valid skills in this plugin.</p>
            )}
            {plugin.skills.map((skill) => (
              <SkillPreview
                key={`${plugin.digest}:${skill.id}`}
                targetId={targetId}
                plugin={plugin}
                skill={skill}
                selected={selections.includes(skill.id)}
                onUseSkill={onUseSkill}
              />
            ))}
            <h4>MCP servers</h4>
            {plugin.mcp_servers.length === 0 ? (
              <p>No MCP servers.</p>
            ) : (
              plugin.mcp_servers.map((server) => (
                <div key={`${plugin.digest}:${server.id}`}>
                  <p>
                    <strong>{server.id}</strong> · {server.transport} ·{" "}
                    {server.status}
                  </p>
                  {inventory?.managementAvailable && (
                    <PluginMcpControls
                      targetId={targetId}
                      server={server.id}
                      enabled={plugin.available && server.enabled}
                      http={
                        server.transport === "http" ||
                        server.transport === "streamable_http"
                      }
                    />
                  )}
                </div>
              ))
            )}
            <p>
              Enable individual MCP servers explicitly in plugin settings.
              Credential configuration does not enable them.
            </p>
            {plugin.diagnostics.map((diagnostic, index) => (
              <p role="status" key={`${diagnostic.code}:${index}`}>
                <strong>
                  {diagnostic.name ?? diagnostic.kind}: {diagnostic.code}
                </strong>{" "}
                — {diagnostic.detail}
              </p>
            ))}
          </article>
        ) : (
          <p>Select a plugin to inspect its skills and resources.</p>
        )}
      </div>
    </section>
  );
}

function SkillPreview({
  targetId,
  plugin,
  skill,
  selected,
  onUseSkill,
}: {
  targetId: string;
  plugin: PluginEntry;
  skill: PluginSkill;
  selected: boolean;
  onUseSkill?: ((id: string) => void) | undefined;
}) {
  const [instructions, setInstructions] = useState<string | null>(null);
  const [resources, setResources] = useState<PluginResource[] | null>(null);
  const [preview, setPreview] = useState<{
    path: string;
    content: string;
  } | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  async function load(kind: "skill" | "resources" | "resource", path?: string) {
    if (loading) return;
    setLoading(true);
    setError("");
    try {
      const request = {
        kind,
        skillId: skill.id,
        digest: plugin.digest,
        ...(path === undefined ? {} : { path }),
      };
      if (kind === "skill")
        setInstructions(
          (await readPluginPreview<{ instructions: string }>(targetId, request))
            .instructions,
        );
      else if (kind === "resources")
        setResources(
          await readPluginPreview<PluginResource[]>(targetId, request),
        );
      else
        setPreview(
          await readPluginPreview<{ path: string; content: string }>(
            targetId,
            request,
          ),
        );
    } catch (error) {
      setError(failure(error));
    } finally {
      setLoading(false);
    }
  }
  return (
    <section className="plugin-skill">
      <h5>{skill.id}</h5>
      <p>{skill.description}</p>
      <div className="plugin-actions">
        <button
          className="button secondary"
          disabled={loading}
          onClick={() => void load("skill")}
        >
          Preview instructions
        </button>
        <button
          className="button secondary"
          disabled={loading}
          onClick={() => void load("resources")}
        >
          Browse resources
        </button>
        <button
          className="button primary"
          disabled={!plugin.available || selected || !onUseSkill}
          onClick={() => onUseSkill?.(skill.id)}
        >
          {selected
            ? "Selected for this conversation"
            : "Use in this conversation"}
        </button>
      </div>
      <small>One message: @{skill.id}</small>
      {loading && <p role="status">Loading bounded preview…</p>}
      {error && <p role="alert">{error}</p>}
      {instructions !== null && (
        <details open>
          <summary>Instructions</summary>
          <pre>{instructions}</pre>
        </details>
      )}
      {resources !== null && (
        <ul aria-label={`${skill.id} resources`}>
          {resources.length === 0 && <li>No resources.</li>}
          {resources.map((resource) => (
            <li key={resource.path}>
              {resource.text ? (
                <button
                  className="button secondary"
                  disabled={loading}
                  onClick={() => void load("resource", resource.path)}
                >
                  {resource.path}
                </button>
              ) : (
                <code>{resource.path}</code>
              )}{" "}
              · {resource.size} bytes
              {!resource.text && " · Binary or oversized; path only"}
            </li>
          ))}
        </ul>
      )}
      {preview && (
        <details open>
          <summary>{preview.path}</summary>
          <pre>{preview.content}</pre>
        </details>
      )}
    </section>
  );
}

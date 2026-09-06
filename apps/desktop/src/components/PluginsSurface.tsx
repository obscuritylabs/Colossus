import { useCallback, useEffect, useRef, useState } from "react";
import {
  cancelPluginOperation,
  getPluginInventory,
  managePlugin,
} from "../api";
import { filterPlugins } from "../plugins";
import type { PluginEntry, PluginInventory, PluginRequest } from "../plugins";
import { PluginOperationForm } from "./PluginOperationForm";
import { PluginDetail } from "./PluginDetail";
import { PluginIcon } from "./PluginIcon";
import {
  IconSearch,
  IconPlus,
  IconRefresh,
  IconPuzzle,
  IconChevronRight,
} from "@tabler/icons-react";
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
  const [filter, setFilter] = useState<"all" | "available" | "unavailable">(
    "all",
  );
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
  const plugins = inventory?.plugins ?? [];
  const visible = filterPlugins(plugins, query).filter(
    (entry) =>
      filter === "all" ||
      (filter === "available" ? entry.available : !entry.available),
  );
  const plugin =
    visible.find((entry) => entry.digest === selected) ?? visible[0];
  const availableCount = plugins.filter((entry) => entry.available).length;
  return (
    <section className="overview-scroll plugin-surface" aria-label="Plugins">
      <header className="plugin-page-header">
        <div>
          <span className="plugin-eyebrow">Your workspace</span>
          <h2>Plugins</h2>
          <p>Give Colossus skills and connections for the way you work.</p>
        </div>
        {inventory?.managementAvailable && (
          <button
            className="button primary"
            disabled={busy !== null}
            onClick={() => setOperation({ action: "install" })}
          >
            <IconPlus size={17} aria-hidden="true" />
            Install
          </button>
        )}
      </header>
      <div className="plugin-toolbar">
        <label className="plugin-search">
          <span className="sr-only">Search plugins and skills</span>
          <IconSearch size={18} aria-hidden="true" />
          <input
            type="search"
            placeholder="Search plugins and skills…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <button
          className="button secondary"
          aria-label="Refresh plugins"
          title="Refresh plugins"
          disabled={loading}
          onClick={() => void refresh()}
        >
          <IconRefresh size={17} aria-hidden="true" />
        </button>
        {inventory?.managementAvailable && (
          <details className="plugin-tools">
            <summary>Developer tools</summary>
            <div className="plugin-actions" aria-label="Plugin management">
              {(
                ["validate", "verify", "package", "pull", "push", "gc"] as const
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
          </details>
        )}
      </div>
      <div className="plugin-filters" aria-label="Filter plugins">
        {(
          [
            ["all", "All plugins", plugins.length],
            ["available", "Available", availableCount],
            ["unavailable", "Unavailable", plugins.length - availableCount],
          ] as const
        ).map(([value, label, count]) => (
          <button
            key={value}
            type="button"
            aria-pressed={filter === value}
            onClick={() => setFilter(value)}
          >
            {label}
            <span>{count}</span>
          </button>
        ))}
      </div>
      {inventory && !inventory.managementAvailable && (
        <p className="plugin-notice">
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
      {inventory && visible.length === 0 && (
        <div className="plugin-empty" role="status">
          <IconPuzzle size={32} stroke={1.4} aria-hidden="true" />
          <h3>
            {plugins.length === 0
              ? "Your plugins will appear here"
              : "No matching plugins"}
          </h3>
          <p>
            {plugins.length === 0
              ? "Install a plugin to add skills and connections to Colossus."
              : "Try another search or show all plugins."}
          </p>
          {plugins.length > 0 && (
            <button
              className="button secondary"
              onClick={() => {
                setQuery("");
                setFilter("all");
              }}
            >
              Clear filters
            </button>
          )}
        </div>
      )}
      <div className="plugin-layout">
        <div className="plugin-inventory" aria-label="Installed plugins">
          {visible.map((entry) => (
            <button
              type="button"
              className="plugin-card"
              key={entry.digest}
              aria-pressed={plugin?.digest === entry.digest}
              onClick={() => setSelected(entry.digest)}
            >
              <div className="plugin-card-heading">
                <PluginIcon
                  name={entry.manifest.name}
                  icon={entry.icon_data_url}
                />
                <span className="plugin-card-title">
                  <strong>{entry.manifest.name}</strong>
                  <span className="plugin-version">
                    {entry.manifest.version ?? "Unversioned"}
                  </span>
                </span>
                <IconChevronRight size={16} aria-hidden="true" />
              </div>
              <span className="plugin-card-description">
                {entry.manifest.description}
              </span>
              <span className="plugin-card-capabilities">
                {entry.skills.length}{" "}
                {entry.skills.length === 1 ? "skill" : "skills"} ·{" "}
                {entry.mcp_servers.length}{" "}
                {entry.mcp_servers.length === 1 ? "connection" : "connections"}
              </span>
              <span className="plugin-card-footer">
                <span
                  className={`plugin-status ${entry.available ? "is-available" : ""}`}
                >
                  {entry.available ? "Available" : "Unavailable"}
                </span>
                <span>
                  {entry.origin === "bundled"
                    ? "Bundled with Colossus"
                    : "Installed"}
                </span>
              </span>
            </button>
          ))}
        </div>
        {plugin ? (
          <PluginDetail
            key={plugin.digest}
            plugin={plugin}
            targetId={targetId}
            managementAvailable={inventory?.managementAvailable === true}
            selections={selections}
            onUseSkill={onUseSkill}
            busy={busy !== null}
            onOperation={setOperation}
          />
        ) : null}
      </div>
    </section>
  );
}

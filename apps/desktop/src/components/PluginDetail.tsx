import { useState } from "react";
import { readPluginPreview } from "../api";
import type { PluginEntry, PluginResource, PluginSkill } from "../plugins";
import { PluginMcpControls } from "./PluginMcpControls";
import { PluginIcon } from "./PluginIcon";
import type { PluginAction } from "./PluginOperationForm";

function failure(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "The plugin request failed. Please retry.";
}

export function PluginDetail({
  plugin,
  targetId,
  managementAvailable,
  selections,
  onUseSkill,
  busy,
  onOperation,
}: {
  plugin: PluginEntry;
  targetId: string;
  managementAvailable: boolean;
  selections: readonly string[];
  onUseSkill: ((id: string) => void) | undefined;
  busy: boolean;
  onOperation: (operation: {
    action: PluginAction;
    plugin: PluginEntry;
  }) => void;
}) {
  return (
    <article
      className="plugin-detail"
      aria-label={`${plugin.manifest.name} details`}
    >
      <header className="plugin-detail-header">
        <PluginIcon
          name={plugin.manifest.name}
          icon={plugin.icon_data_url}
          size="large"
        />
        <div>
          <span className="plugin-eyebrow">
            {plugin.origin === "bundled"
              ? "Bundled with Colossus"
              : "Installed plugin"}
          </span>
          <h3>{plugin.manifest.name}</h3>
          <span className="plugin-version">
            {plugin.manifest.version ?? "Unversioned"}
          </span>
        </div>
        <span
          className={`plugin-status ${plugin.available ? "is-available" : ""}`}
        >
          {plugin.available ? "Available" : "Unavailable"}
        </span>
      </header>
      <p className="plugin-description">{plugin.manifest.description}</p>
      {!plugin.available && (
        <p role="status">
          {plugin.unavailable_reason ?? "Unavailable in this workspace"}
        </p>
      )}
      {managementAvailable && (
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
                disabled={busy}
                onClick={() =>
                  onOperation({
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
      <div className="plugin-section-heading">
        <h4>
          Skills <span>{plugin.skills.length}</span>
        </h4>
        <p>Select a skill for your conversation, or mention it with @.</p>
      </div>
      {plugin.skills.length === 0 && <p>No valid skills in this plugin.</p>}
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
      <div className="plugin-section-heading">
        <h4>
          MCP servers <span>{plugin.mcp_servers.length}</span>
        </h4>
        <p>Connections provided by this plugin.</p>
      </div>
      {plugin.mcp_servers.length === 0 ? (
        <p>No MCP servers.</p>
      ) : (
        plugin.mcp_servers.map((server) => (
          <div className="plugin-server" key={`${plugin.digest}:${server.id}`}>
            <p>
              <strong>{server.id}</strong> · {server.transport} ·{" "}
              {server.status}
            </p>
            {managementAvailable && (
              <PluginMcpControls
                targetId={targetId}
                server={server.id}
                enabled={plugin.available && server.enabled}
                http={
                  server.transport === "http" ||
                  server.transport === "streamable-http" ||
                  server.transport === "streamable_http"
                }
              />
            )}
          </div>
        ))
      )}
      <p>
        Enable individual MCP servers explicitly in plugin settings. Credential
        configuration does not enable them.
      </p>
      <details className="plugin-installation">
        <summary>Installation details</summary>
        <dl>
          <dt>Source</dt>
          <dd>
            {plugin.origin === "bundled"
              ? "Bundled with Colossus"
              : plugin.source}
          </dd>
          <dt>Global state</dt>
          <dd>{plugin.status}</dd>
          <dt>Trust</dt>
          <dd>
            {plugin.origin === "bundled"
              ? "Bundled with Colossus. Version managed by the executable; this is not a Cosign signature."
              : `${plugin.trust.trusted ? "Signature verified" : "Untrusted"} · ${plugin.trust.method} · ${plugin.trust.profile ?? "No trust profile"}`}
          </dd>
          {plugin.trust.signer && (
            <>
              <dt>Signer</dt>
              <dd>{plugin.trust.signer}</dd>
            </>
          )}
          <dt>Manifest digest</dt>
          <dd className="plugin-digest">
            <code>{plugin.digest}</code>
          </dd>
        </dl>
      </details>
      {plugin.diagnostics.map((diagnostic, index) => (
        <p role="status" key={`${diagnostic.code}:${index}`}>
          <strong>
            {diagnostic.name ?? diagnostic.kind}: {diagnostic.code}
          </strong>{" "}
          — {diagnostic.detail}
        </p>
      ))}
    </article>
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

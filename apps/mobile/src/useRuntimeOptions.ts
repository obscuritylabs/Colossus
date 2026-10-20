import { useEffect, useState } from "react";
import type { FleetNode } from "@colossus/control-plane-client";
import { messageOf, type MobileClient } from "./client";

export interface ConfiguredModel {
  profile: string;
  model: string;
  provider: string;
  provider_profile: string;
  reasoning_effort: string | null;
  default_roles: string[];
}
interface ResourceReply {
  connection_id?: string;
  kind: string;
  value?: { capabilities?: unknown; model_profiles?: unknown };
}
function models(value: unknown): ConfiguredModel[] {
  return Array.isArray(value)
    ? value
        .filter((entry): entry is ConfiguredModel =>
          Boolean(
            entry &&
            typeof entry === "object" &&
            typeof entry.profile === "string" &&
            entry.profile &&
            typeof entry.model === "string" &&
            typeof entry.provider === "string" &&
            typeof entry.provider_profile === "string" &&
            (entry.reasoning_effort == null ||
              typeof entry.reasoning_effort === "string") &&
            Array.isArray(entry.default_roles) &&
            entry.default_roles.every(
              (role: unknown) => typeof role === "string",
            ),
          ),
        )
        .slice(0, 256)
    : [];
}
/** Catalogs and capabilities belong to the exact authenticated runtime connection. */
export function useRuntimeOptions(
  client: MobileClient,
  project: string,
  target: FleetNode | undefined,
  canExecute: boolean,
) {
  const node = target?.node.node_id ?? "",
    connection = target?.presence?.connection_id ?? "";
  const ready = Boolean(target?.presence?.ready && !target.node.revoked);
  const scope = JSON.stringify([project, node, connection, canExecute]);
  const [state, setState] = useState<{
    scope: string;
    capabilities: string[];
    models: ConfiguredModel[];
    error: string;
    loading: boolean;
  }>({ scope: "", capabilities: [], models: [], error: "", loading: false });
  useEffect(() => {
    let alive = true;
    if (!ready || !node || !connection) return;
    setState({ scope, capabilities: [], models: [], error: "", loading: true });
    void client
      .request<ResourceReply>({
        kind: "runtime_context",
        project,
        node,
        connection_id: connection,
      })
      .then(async (reply) => {
        if (!alive) return;
        const valid =
          reply.kind === "result" && reply.connection_id === connection;
        const capabilities =
          valid && Array.isArray(reply.value?.capabilities)
            ? reply.value.capabilities.filter(
                (item): item is string => typeof item === "string",
              )
            : [];
        setState({
          scope,
          capabilities,
          models: [],
          error: "",
          loading:
            canExecute &&
            capabilities.includes("runtime.model_catalog.v1") &&
            capabilities.includes("agent_runs.model_selection"),
        });
        if (
          !canExecute ||
          !capabilities.includes("runtime.model_catalog.v1") ||
          !capabilities.includes("agent_runs.model_selection")
        )
          return;
        try {
          const catalog = await client.request<ResourceReply>({
            kind: "model_catalog",
            project,
            node,
            connection_id: connection,
          });
          if (!alive) return;
          setState({
            scope,
            capabilities,
            models:
              catalog.kind === "result" && catalog.connection_id === connection
                ? models(catalog.value?.model_profiles)
                : [],
            error: "",
            loading: false,
          });
        } catch (failure) {
          if (alive)
            setState({
              scope,
              capabilities,
              models: [],
              error: messageOf(failure),
              loading: false,
            });
        }
      })
      .catch((failure: unknown) => {
        if (alive)
          setState({
            scope,
            capabilities: [],
            models: [],
            error: messageOf(failure),
            loading: false,
          });
      });
    return () => {
      alive = false;
    };
  }, [client, scope, project, node, connection, ready, canExecute]);
  const current = ready && state.scope === scope;
  return {
    scope,
    capabilities: current ? state.capabilities : [],
    models: current ? state.models : [],
    error: current ? state.error : "",
    loading: ready && (!current || state.loading),
  };
}

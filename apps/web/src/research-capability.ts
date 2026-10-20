import { useEffect, useState } from "react";
import { projectPath, request, type FleetNode } from "./api";
import { configuredModels, type ConfiguredModel } from "./model-catalog";

interface ResourceReply {
  connection_id?: string;
  kind: string;
  value?: { capabilities?: unknown; model_profiles?: unknown };
}

/** Capability facts stay in the host and belong to the exact authenticated connection. */
export function useRunModeCapabilities(
  target: FleetNode | undefined,
  disabled: boolean,
) {
  const scope = target?.presence
    ? JSON.stringify([
        target.node.project_id,
        target.node.node_id,
        target.presence.connection_id,
      ])
    : "";
  const ready = Boolean(target?.presence?.ready && !target.node.revoked);
  const advertisedResearch = Boolean(
    target?.presence?.capabilities.includes("research.create"),
  );
  const advertisedGoal = Boolean(
    target?.presence?.capabilities.includes("goal.create"),
  );
  const advertisedPlans = Boolean(
    target?.presence?.capabilities.includes("plans.continue"),
  );
  const advertisedModels = Boolean(
    target?.presence?.capabilities.includes("runtime.model_catalog.v1") &&
    target.presence.capabilities.includes("agent_runs.model_selection"),
  );
  const modelScope = JSON.stringify([
    scope,
    ready,
    !disabled,
    advertisedModels,
  ]);
  const [state, setState] = useState<{
    scope: string;
    modelScope: string;
    research: boolean;
    goal: boolean;
    plans: boolean;
    modelProfiles: ConfiguredModel[];
    modelLoading: boolean;
  }>({
    scope: "",
    modelScope: "",
    research: false,
    goal: false,
    plans: false,
    modelProfiles: [],
    modelLoading: false,
  });
  useEffect(() => {
    const empty = {
      scope,
      modelScope,
      research: false,
      goal: false,
      plans: false,
      modelProfiles: [],
      modelLoading: false,
    };
    setState(empty);
    if (
      !target ||
      !ready ||
      disabled ||
      (advertisedResearch && advertisedGoal && !advertisedModels)
    )
      return;
    const abort = new AbortController();
    const connection = target.presence!.connection_id;
    const path = `${projectPath(target.node.project_id)}/nodes/${encodeURIComponent(target.node.node_id)}/resources`;
    setState({ ...empty, modelLoading: advertisedModels });
    void request<ResourceReply>(
      path,
      {
        connection_id: connection,
        operation: { operation: "context" },
      },
      abort.signal,
    )
      .then(async (reply) => {
        if (abort.signal.aborted) return;
        const capabilities = reply.value?.capabilities;
        const valid =
          reply.kind === "result" &&
          reply.connection_id === connection &&
          Array.isArray(capabilities);
        const modelsAvailable =
          advertisedModels &&
          valid &&
          capabilities.includes("runtime.model_catalog.v1") &&
          capabilities.includes("agent_runs.model_selection");
        const context = {
          ...empty,
          research: valid && capabilities.includes("research.create"),
          goal: valid && capabilities.includes("goal.create"),
          plans: valid && capabilities.includes("plans.continue"),
          modelLoading: modelsAvailable,
        };
        setState(context);
        if (!modelsAvailable) return;
        try {
          const catalog = await request<ResourceReply>(
            path,
            {
              connection_id: connection,
              operation: { operation: "model_catalog" },
            },
            abort.signal,
          );
          if (!abort.signal.aborted)
            setState({
              ...context,
              modelProfiles:
                catalog.kind === "result" &&
                catalog.connection_id === connection
                  ? configuredModels(catalog.value?.model_profiles)
                  : [],
              modelLoading: false,
            });
        } catch {
          if (!abort.signal.aborted)
            setState({ ...context, modelLoading: false });
        }
      })
      .catch(() => {
        if (!abort.signal.aborted) setState(empty);
      });
    return () => abort.abort();
  }, [
    scope,
    modelScope,
    ready,
    disabled,
    advertisedResearch,
    advertisedGoal,
    advertisedPlans,
    advertisedModels,
  ]);
  const current = ready && !disabled;
  const resolved = state.scope === scope;
  return {
    research: current && (advertisedResearch || (resolved && state.research)),
    goal: current && (advertisedGoal || (resolved && state.goal)),
    plans: current && (advertisedPlans || (resolved && state.plans)),
    modelScope,
    modelProfiles:
      current && state.modelScope === modelScope ? state.modelProfiles : [],
    modelLoading:
      current &&
      advertisedModels &&
      (state.modelScope !== modelScope || state.modelLoading),
  };
}

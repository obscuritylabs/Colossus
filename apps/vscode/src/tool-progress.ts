import {
  ToolActivityState,
  toolActivityStateToJSON,
  type RunUpdate,
} from "@obscuritylabs/colossus-sdk/gen/colossus/api/v1alpha1/agent_run";
import type { ToolView } from "./model.js";

// Only policy-released ToolActivity fields cross into the webview. Decimal
// sequences retain uint64 precision; call identities are scoped to their run.
export function toolProgress(tools: ToolView[], item: RunUpdate): ToolView[] {
  if (item.update?.$case !== "toolActivity") return tools;
  const tool = item.update.value;
  const id = tool.callId || `sequence-${item.sequence}`;
  const index = tools.findIndex((t) => t.runId === item.runId && t.id === id);
  const previous = index < 0 ? undefined : tools[index];
  if (previous?.sequence && BigInt(previous.sequence) >= item.sequence)
    return tools;
  const at = item.createdAt?.toISOString() ?? "";
  const state = toolActivityStateToJSON(tool.state)
    .replace("TOOL_ACTIVITY_STATE_", "")
    .toLowerCase()
    .replaceAll("_", " ");
  const summary = tool.summary.slice(0, 2048);
  const next: ToolView = {
    id,
    runId: item.runId,
    name: tool.toolName.slice(0, 256),
    state,
    summary,
    sequence: item.sequence.toString(),
    startedAt: previous?.startedAt || at,
    updatedAt: at,
    history: [...(previous?.history ?? []), { state, summary, at }].slice(-8),
  };
  const input = tool.input ?? previous?.input;
  if (input !== undefined) next.input = input.slice(0, 8192);
  // A preview never carries across a later failed/uncertain lifecycle update.
  if (
    tool.state === ToolActivityState.TOOL_ACTIVITY_STATE_COMPLETED &&
    tool.preview !== undefined
  )
    next.preview = tool.preview.slice(0, 8192);
  const result = [...tools];
  if (index < 0) result.push(next);
  else result[index] = next;
  return result.slice(-100);
}

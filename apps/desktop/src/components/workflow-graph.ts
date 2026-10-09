import { MarkerType, type Edge, type Node } from "@xyflow/react";
import type {
  WorkflowLogic,
  WorkflowLogicStep,
  WorkflowStepState,
} from "../workflows";

export const STEP_WIDTH = 220;
export const STEP_HEIGHT = 82;
const GAP = 28;
const LANE_GAP = 48;
const JOIN_HEIGHT = 18;

export type LogicNodeData = {
  label: string;
  step?: WorkflowLogicStep;
  state?: WorkflowStepState | undefined;
  selected?: boolean;
  onSelect?: (id: string) => void;
  onFocus?: (id: string) => void;
};
export type LogicNode = Node<
  LogicNodeData,
  "workflowStep" | "workflowBoundary" | "workflowJoin"
>;
type Size = { width: number; height: number };
const sequenceSize = (steps: WorkflowLogicStep[]): Size => {
  const sizes = steps.map(stepSize);
  return {
    width: Math.max(STEP_WIDTH, ...sizes.map((size) => size.width)),
    height:
      sizes.reduce((sum, size) => sum + size.height, 0) +
      Math.max(0, steps.length - 1) * GAP,
  };
};
const stepSize = (step: WorkflowLogicStep): Size => {
  if (!step.branches.length) return { width: STEP_WIDTH, height: STEP_HEIGHT };
  const sizes = step.branches.map((branch) => sequenceSize(branch.steps));
  return {
    width:
      sizes.reduce((sum, size) => sum + size.width, 0) +
      (sizes.length - 1) * LANE_GAP,
    height:
      STEP_HEIGHT +
      GAP +
      Math.max(...sizes.map((size) => size.height)) +
      GAP +
      JOIN_HEIGHT,
  };
};

export function logicSteps(logic: WorkflowLogic): WorkflowLogicStep[] {
  const result: WorkflowLogicStep[] = [];
  const visit = (steps: WorkflowLogicStep[]) => {
    for (const step of steps) {
      result.push(step);
      step.branches.forEach((branch) => visit(branch.steps));
    }
  };
  visit(logic.steps);
  visit(logic.compensation);
  return result;
}

/** Lay out structural branches independently from execution state; never infer a route. */
export function buildWorkflowGraph(logic: WorkflowLogic): {
  nodes: LogicNode[];
  edges: Edge[];
} {
  const nodes: LogicNode[] = [];
  const edges: Edge[] = [];
  const connect = (
    source: string,
    target: string,
    label?: string,
    loop = false,
    bypass = false,
  ) => {
    edges.push({
      id: `edge:${edges.length}`,
      source,
      target,
      label,
      type: "smoothstep",
      markerEnd: {
        type: MarkerType.ArrowClosed,
        width: 14,
        height: 14,
        color: loop ? "var(--muted)" : "var(--blue)",
      },
      ...(loop ? { sourceHandle: "loop-out", targetHandle: "loop-in" } : {}),
      ...(bypass
        ? { sourceHandle: "bypass-out", targetHandle: "bypass-in" }
        : {}),
      data: { loop },
      style: {
        stroke: loop ? "var(--muted)" : "var(--blue)",
        strokeWidth: 1.6,
        ...(loop ? { strokeDasharray: "5 4" } : {}),
      },
      labelStyle: { fill: "var(--text)", fontSize: 12, fontWeight: 600 },
      labelBgStyle: { fill: "var(--surface)" },
      labelBgPadding: [6, 4],
      focusable: false,
      selectable: false,
    });
  };
  function sequence(
    steps: WorkflowLogicStep[],
    x: number,
    y: number,
  ): { first: string; last: string } | null {
    let first: string | null = null;
    let last: string | null = null;
    const width = sequenceSize(steps).width;
    for (const step of steps) {
      const size = stepSize(step);
      const center = x + width / 2;
      const id = `step:${step.id}`;
      nodes.push({
        id,
        type: "workflowStep",
        position: { x: center - STEP_WIDTH / 2, y },
        data: { label: step.id.replaceAll(/[-_]/g, " "), step },
        style: { width: STEP_WIDTH, height: STEP_HEIGHT },
      });
      if (last) connect(last, id);
      first ??= id;
      last = id;
      if (step.branches.length) {
        const joinId = `join:${step.id}`;
        const joinY = y + size.height - JOIN_HEIGHT;
        nodes.push({
          id: joinId,
          type: "workflowJoin",
          position: { x: center - 9, y: joinY },
          data: { label: "Continue" },
          style: { width: 18, height: JOIN_HEIGHT },
        });
        let branchX = center - size.width / 2;
        for (const branch of step.branches) {
          const branchSize = sequenceSize(branch.steps);
          const ends = sequence(branch.steps, branchX, y + STEP_HEIGHT + GAP);
          if (ends) {
            connect(id, ends.first, branch.label);
            if (step.kind === "foreach")
              connect(ends.last, id, "Next item", true);
            else connect(ends.last, joinId);
          } else {
            connect(id, joinId, branch.label);
          }
          branchX += branchSize.width + LANE_GAP;
        }
        if (step.kind === "foreach") connect(id, joinId, "Done", false, true);
        last = joinId;
      }
      y += size.height + GAP;
    }
    return first && last ? { first, last } : null;
  }
  function lane(
    steps: WorkflowLogicStep[],
    x: number,
    label: string,
    prefix: string,
  ) {
    const size = sequenceSize(steps);
    const center = x + size.width / 2;
    const start = `${prefix}:start`;
    const end = `${prefix}:end`;
    nodes.push({
      id: start,
      type: "workflowBoundary",
      position: { x: center - 75, y: 0 },
      data: { label },
      style: { width: 150, height: 36 },
    });
    const ends = sequence(steps, x, 36 + GAP);
    nodes.push({
      id: end,
      type: "workflowBoundary",
      position: { x: center - 75, y: size.height + 36 + 2 * GAP },
      data: { label: prefix === "main" ? "Finish" : "Recovery finished" },
      style: { width: 150, height: 36 },
    });
    if (ends) {
      connect(start, ends.first);
      connect(ends.last, end);
    }
  }
  lane(logic.steps, 0, "Start", "main");
  if (logic.compensation.length)
    lane(
      logic.compensation,
      sequenceSize(logic.steps).width + LANE_GAP * 2,
      "On failure · recovery",
      "recovery",
    );
  return { nodes, edges };
}

export const stepKindLabel = (kind: WorkflowLogicStep["kind"]): string =>
  ({
    agent: "Agent",
    tool: "Tool",
    workflow: "Child workflow",
    approval: "Approval",
    condition: "Condition",
    parallel: "Parallel",
    foreach: "For each",
    wait_for_input: "Wait for input",
    emit: "Output",
  })[kind];

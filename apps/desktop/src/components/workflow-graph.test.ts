import { describe, expect, it } from "vitest";
import type { WorkflowLogic, WorkflowLogicStep } from "../workflows";
import { buildWorkflowGraph } from "./workflow-graph";

const output = (id: string): WorkflowLogicStep => ({
  id,
  kind: "emit",
  summary: "Output",
  branches: [],
});
const logic: WorkflowLogic = {
  steps: [
    {
      id: "route",
      kind: "condition",
      summary: "/inputs/production == true",
      branches: [
        { label: "True", steps: [output("first"), output("second")] },
        { label: "False", steps: [] },
      ],
    },
    {
      id: "loop",
      kind: "foreach",
      summary: "/inputs/items · at most 4 items",
      branches: [
        {
          label: "Each item",
          steps: [output("item-first"), output("item-last")],
        },
      ],
    },
    output("result"),
  ],
  compensation: [output("recover")],
};

describe("workflow logic paths", () => {
  it("preserves branch sequencing, empty alternatives, and the loop decision before exit", () => {
    const { nodes, edges } = buildWorkflowGraph(logic);
    const outgoing = (id: string) => edges.filter((edge) => edge.source === id);
    expect(
      outgoing("step:route").map((edge) => [edge.label, edge.target]),
    ).toEqual([
      ["True", "step:first"],
      ["False", "join:route"],
    ]);
    expect(outgoing("step:first")[0]?.target).toBe("step:second");
    expect(outgoing("step:second")[0]?.target).toBe("join:route");
    expect(outgoing("join:route")[0]?.target).toBe("step:loop");
    expect(
      outgoing("step:loop").map((edge) => [edge.label, edge.target]),
    ).toEqual([
      ["Each item", "step:item-first"],
      ["Done", "join:loop"],
    ]);
    expect(
      outgoing("step:item-last").map((edge) => [edge.label, edge.target]),
    ).toEqual([["Next item", "step:loop"]]);
    expect(outgoing("join:loop")[0]?.target).toBe("step:result");
    expect(new Set(nodes.map((node) => node.id)).size).toBe(nodes.length);
  });

  it("keeps recovery separate and never invents execution state from the definition", () => {
    const { nodes, edges } = buildWorkflowGraph(logic);
    const reachable = new Set(["main:start"]);
    for (let iteration = 0; iteration < nodes.length; iteration++) {
      edges.forEach((edge) => {
        if (reachable.has(edge.source)) reachable.add(edge.target);
      });
    }
    expect(reachable.has("main:end")).toBe(true);
    expect(reachable.has("step:recover")).toBe(false);
    expect(nodes.every((node) => node.data.state === undefined)).toBe(true);
  });
});

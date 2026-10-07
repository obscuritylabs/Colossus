import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ModelRoutingOverview, modelRouteGroups } from "./ModelRoutingOverview";

const models = [
  {
    profile: "general",
    label: "General",
    model: "general-model",
    providerProfile: "company",
  },
  {
    profile: "fast",
    label: "Fast",
    model: "fast-model",
    providerProfile: "local",
  },
];

describe("model routing map", () => {
  it("moves inherited routes with Primary and preserves explicit assignments", () => {
    const before = modelRouteGroups(
      { primary: "general", research_planner: "fast" },
      models,
    );
    expect(
      before.find((group) => group.profile === "general")?.roles,
    ).toHaveLength(6);
    expect(before.find((group) => group.profile === "fast")?.roles).toEqual([
      { id: "research_planner", label: "Research planner", inherited: false },
    ]);
    const after = modelRouteGroups(
      { primary: "fast", research_planner: "general" },
      models,
    );
    expect(after.find((group) => group.profile === "fast")?.roles).toHaveLength(
      6,
    );
    expect(
      after.find((group) => group.profile === "general")?.roles[0]?.inherited,
    ).toBe(false);
  });

  it("shows an unavailable explicit target instead of silently using Primary", () => {
    const groups = modelRouteGroups(
      { primary: "general", risk_evaluator: "removed" },
      models,
    );
    const missing = groups.find((group) => group.profile === "removed");
    expect(missing?.model).toBeUndefined();
    expect(missing?.roles).toEqual([
      { id: "risk_evaluator", label: "Risk evaluator", inherited: false },
    ]);
    const markup = renderToStaticMarkup(
      createElement(ModelRoutingOverview, {
        roles: { primary: "general", risk_evaluator: "removed" },
        models,
      }),
    );
    expect(markup).toContain("Routing map");
  });

  it("leaves inherited routes unresolved when Primary has no model", () => {
    const groups = modelRouteGroups({ research_worker: "fast" }, models);
    expect(groups.find((group) => group.profile === "")?.roles).toHaveLength(6);
    expect(groups.find((group) => group.profile === "")?.model).toBeUndefined();
    expect(groups.find((group) => group.profile === "fast")?.roles[0]?.id).toBe(
      "research_worker",
    );
  });
});

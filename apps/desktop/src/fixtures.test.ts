import { afterEach, describe, expect, it, vi } from "vitest";

import { buildOperationsStudioFixture } from "./dev/operations-studio-fixture";
import { MAX_FEED_ITEMS, MAX_RECENT_RUNS } from "./state";

describe("buildOperationsStudioFixture", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("builds a deterministic selected work item with the full operations feed", () => {
    const first = buildOperationsStudioFixture();
    const second = buildOperationsStudioFixture();

    expect(first).toEqual(second);
    expect(first.activeRunId).not.toBeNull();

    const selected = first.views.get(first.activeRunId ?? "");
    expect(selected?.run.status).toBe("waiting");
    expect(selected?.updates.map(({ update }) => update.type)).toEqual([
      "message",
      "reasoning_summary",
      "tool_activity",
      "tool_activity",
      "notice",
      "message",
      "tool_activity",
      "interaction",
      "usage",
    ]);
    expect(selected?.seenSequences.size).toBe(selected?.lastSequence);
    expect(selected?.usage?.totalTokens).toBe(
      (selected?.usage?.inputTokens ?? 0) +
        (selected?.usage?.outputTokens ?? 0),
    );
  });

  it("includes a respondable approval and available input and output artifacts", () => {
    const fixture = buildOperationsStudioFixture();
    const selected = fixture.views.get(fixture.activeRunId ?? "");
    const approval = selected?.pendingInteractions[0];

    expect(approval).toMatchObject({
      kind: "approval",
      status: "pending",
      respondableByCaller: true,
    });
    expect(approval?.content.type).toBe("approval");
    expect(selected?.run.pendingInteractionCount).toBe(1);

    const artifacts =
      selected?.updates.flatMap(({ update }) =>
        update.type === "message"
          ? update.message.content.flatMap((part) =>
              part.type === "artifact" ? [part.artifact] : [],
            )
          : [],
      ) ?? [];
    expect(artifacts).toHaveLength(3);
    expect(artifacts.map(({ purpose }) => purpose)).toEqual([
      "run_output",
      "run_output",
      "run_input",
    ]);
    expect(artifacts.every(({ state }) => state === "available")).toBe(true);
  });

  it("returns bounded fresh collections", () => {
    const first = buildOperationsStudioFixture();
    const second = buildOperationsStudioFixture();
    const selected = first.views.get(first.activeRunId ?? "");

    expect(first.recentRuns.length).toBeLessThanOrEqual(MAX_RECENT_RUNS);
    expect(selected?.updates.length).toBeLessThanOrEqual(MAX_FEED_ITEMS);
    expect(first.views).not.toBe(second.views);
    expect(selected?.seenSequences).not.toBe(
      second.views.get(second.activeRunId ?? "")?.seenSequences,
    );
  });

  it("refuses to activate fixture state outside development", () => {
    vi.stubEnv("DEV", false);

    expect(() => buildOperationsStudioFixture()).toThrowError(
      "Operations Studio fixtures are available in development only.",
    );
  });
});

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { buildSessionActivityFixture } from "../dev/operations-studio-fixture";
import type { SessionActivity } from "../types";
import {
  SessionActivityView,
  activityGroups,
  mergeActivities,
} from "./SessionActivity";

function fixtureActivities(): SessionActivity[] {
  return buildSessionActivityFixture({
    sourceRunId: "fixture-run-desktop-release",
    pageSize: 100,
  }).activities;
}

describe("SessionActivityView", () => {
  it("renders an explicit upgrade state for older targets", () => {
    const markup = renderToStaticMarkup(
      createElement(SessionActivityView, {
        sourceRunId: "run-one",
        available: false,
        loadPage: vi.fn(),
      }),
    );

    expect(markup).toContain("Activity requires a newer Colossus target");
    expect(markup).toContain("sessions.activity");
  });

  it("groups newest turns first while keeping events chronological", () => {
    const groups = activityGroups(fixtureActivities());

    expect(groups[0]?.turn).toBe(4);
    expect(groups[0]?.activities).toHaveLength(12);
    expect(groups.at(-1)?.turn).toBe(1);
    for (const group of groups) {
      expect(group.activities).toEqual(
        [...group.activities].sort(
          (left, right) => left.firstSequence - right.firstSequence,
        ),
      );
    }
  });

  it("merges live pages by stable ID without duplicating activities", () => {
    const [latest, ...rest] = fixtureActivities();
    if (latest === undefined) throw new Error("fixture must include activity");
    const replacement = {
      ...latest,
      summary: "Updated released summary",
      lastSequence: latest.lastSequence + 1,
    };

    const merged = mergeActivities(rest, [latest, replacement]);

    expect(
      merged.filter((item) => item.activityId === latest.activityId),
    ).toHaveLength(1);
    expect(
      merged.find((item) => item.activityId === latest.activityId)?.summary,
    ).toBe("Updated released summary");
  });

  it("provides realistic filtering and released inspector payloads", () => {
    const page = buildSessionActivityFixture({
      sourceRunId: "fixture-run-desktop-release",
      query: "denied",
      lanes: ["tools"],
      statuses: ["failed"],
      pageSize: 100,
    });

    expect(page.activities).toHaveLength(1);
    expect(page.activities[0]?.summary).toContain("denied");
    expect(page.activities[0]?.result?.value).toContain(
      "not released by policy",
    );
  });
});

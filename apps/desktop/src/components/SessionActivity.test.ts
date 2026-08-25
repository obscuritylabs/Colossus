import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { buildSessionActivityFixture } from "../dev/operations-studio-fixture";
import type { SessionActivity } from "../types";
import {
  SessionActivityView,
  activityGroups,
  clampTimelineRange,
  historyTokenAfterHeadRefresh,
  mergeActivities,
  panTimelineRange,
  timelineExtent,
  zoomTimelineRange,
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

  it("keeps the historical cursor when a live head refresh succeeds", () => {
    expect(historyTokenAfterHeadRefresh("older-page", "new-head", true)).toBe(
      "older-page",
    );
    expect(historyTokenAfterHeadRefresh("older-page", "new-head", false)).toBe(
      "new-head",
    );
  });

  it("derives the timeline extent from released start and completion times", () => {
    const [sample] = fixtureActivities();
    if (sample === undefined) throw new Error("fixture must include activity");
    const startedAt = Date.parse("2026-08-22T12:00:00.000Z");
    const completedAt = Date.parse("2026-08-22T12:02:00.000Z");

    expect(
      timelineExtent([
        {
          ...sample,
          startedAt: new Date(startedAt).toISOString(),
          completedAt: new Date(completedAt).toISOString(),
        },
      ]),
    ).toEqual({ start: startedAt, end: completedAt });
    expect(timelineExtent([], completedAt)).toEqual({
      start: completedAt - 60_000,
      end: completedAt,
    });
  });

  it("clamps panning to the loaded activity extent", () => {
    const extent = { start: 0, end: 100_000 };
    const range = { start: 20_000, end: 50_000 };

    expect(panTimelineRange(range, extent, 90_000)).toEqual({
      start: 70_000,
      end: 100_000,
    });
    expect(panTimelineRange(range, extent, -90_000)).toEqual({
      start: 0,
      end: 30_000,
    });
    expect(
      clampTimelineRange({ start: -10_000, end: 150_000 }, extent),
    ).toEqual(extent);
  });

  it("zooms around the requested time and honors the minimum span", () => {
    const extent = { start: 0, end: 100_000 };
    const range = { start: 20_000, end: 60_000 };

    expect(zoomTimelineRange(range, extent, 0.5, 20_000)).toEqual({
      start: 20_000,
      end: 40_000,
    });
    expect(zoomTimelineRange(range, extent, 100)).toEqual(extent);
    expect(
      zoomTimelineRange({ start: 20_000, end: 21_000 }, extent, 0.01),
    ).toEqual({ start: 20_000, end: 21_000 });
  });

  it("groups delegated runs with their released role and parent lineage", () => {
    const [sample] = fixtureActivities();
    if (sample === undefined) throw new Error("fixture must include activity");
    const [group] = activityGroups([
      {
        ...sample,
        runId: "child-run",
        attributes: {
          run_role: "subagent",
          subagent_role: "security-reviewer",
          parent_run_id: "parent-run",
        },
      },
    ]);

    expect(group?.runRole).toBe("subagent");
    expect(group?.subagentRole).toBe("security-reviewer");
    expect(group?.parentRunId).toBe("parent-run");
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

  it("stages deterministic head records for live-follow acceptance tests", () => {
    const request = {
      sourceRunId: "fixture-run-desktop-release",
      pageSize: 100,
    };

    const initial = buildSessionActivityFixture(request);
    const firstPoll = buildSessionActivityFixture(request, {
      liveActivityCount: 1,
    });
    const secondPoll = buildSessionActivityFixture(request, {
      liveActivityCount: 2,
    });

    expect(initial.activities).toHaveLength(27);
    expect(firstPoll.activities[0]?.title).toBe("Live checkpoint");
    expect(firstPoll.headSequence).toBe(42);
    expect(secondPoll.activities[0]?.title).toBe("Live response");
    expect(secondPoll.headSequence).toBe(43);
  });
});

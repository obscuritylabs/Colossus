import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import {
  ControlPlaneConnectionInventory,
  controlPlaneState,
} from "./ControlPlaneConnections";
import type { ControlPlaneProfiles } from "../types";

const catalog: ControlPlaneProfiles = {
  revision: 1,
  defaultProfile: "bookmark",
  profiles: [
    {
      id: "bookmark",
      label: "Saved team endpoint",
      endpoint: "https://bookmark.example.test",
    },
  ],
  connections: [
    {
      targetId: "target-a",
      status: "connected",
      projectId: "project-a",
      endpoint: "https://live.example.test",
      sharedSessions: false,
    },
    {
      targetId: "space-b",
      status: "disconnected",
      projectId: "project-b",
      endpoint: "https://offline.example.test",
      sharedSessions: false,
    },
    {
      targetId: "missing",
      status: "revoked",
      projectId: "project-removed",
      endpoint: "https://removed.example.test",
      sharedSessions: false,
    },
  ],
};

it("separates actual enrolled workspace state from endpoint bookmarks and missing workspace navigation", () => {
  const markup = renderToStaticMarkup(
    createElement(ControlPlaneConnectionInventory, {
      catalog,
      spaces: [
        {
          spaceId: "space-a",
          targetId: "target-a",
          displayName: "Workspace A",
        },
        {
          spaceId: "space-b",
          targetId: "target-b",
          displayName: "Workspace B",
        },
      ],
      onManageWorkspace: () => undefined,
      onManageProfiles: () => undefined,
    }),
  );
  expect(markup).toContain("Workspace A");
  expect(markup).toContain("Workspace B");
  expect(markup).toContain("Connected");
  expect(markup).toContain("Disconnected");
  expect(markup).toContain("Revoked");
  expect(markup).toContain("project-b");
  expect(markup).toContain("https://offline.example.test");
  expect(markup).toMatch(
    /disabled=""[^>]*aria-label="Manage Control Plane for missing"/,
  );
  const bookmark = markup.slice(
    markup.indexOf('class="control-plane-bookmarks"'),
  );
  expect(bookmark).toContain("Default bookmark");
  expect(bookmark).not.toContain("Connected");
});

it("never turns an unrecognized or unavailable status into Connected", () => {
  expect(controlPlaneState("connected")).toEqual({
    label: "Connected",
    tone: "success",
  });
  expect(controlPlaneState("connecting").label).toBe("Connecting");
  expect(controlPlaneState("reconnecting").label).toBe("Reconnecting");
  expect(controlPlaneState("unexpected").label).toBe("Unknown");
  expect(controlPlaneState("connected", true).label).toBe("Unknown");
});

it("surfaces incomplete sharing changes without changing workspace navigation", () => {
  const markup = renderToStaticMarkup(
    createElement(ControlPlaneConnectionInventory, {
      catalog: {
        ...catalog,
        connections: [
          { ...catalog.connections[1]!, sharingRecoveryRequired: true },
        ],
      },
      spaces: [
        {
          spaceId: "space-b",
          targetId: "target-b",
          displayName: "Workspace B",
        },
      ],
      onManageWorkspace: () => undefined,
      onManageProfiles: () => undefined,
    }),
  );
  expect(markup).toContain("Sharing needs reconciliation");
  expect(markup).toContain("Disconnected");
  expect(markup).toContain('aria-label="Manage Control Plane for Workspace B"');
  expect(markup).not.toMatch(
    /disabled=""[^>]*aria-label="Manage Control Plane for Workspace B"/,
  );
});

it("marks retained enrollment metadata Unknown while keeping endpoint bookmarks usable", () => {
  const markup = renderToStaticMarkup(
    createElement(ControlPlaneConnectionInventory, {
      catalog: { ...catalog, connectionStatusUnavailable: true },
      spaces: [
        {
          spaceId: "space-a",
          targetId: "target-a",
          displayName: "Workspace A",
        },
      ],
      onManageWorkspace: () => undefined,
      onManageProfiles: () => undefined,
    }),
  );
  expect(markup).toContain("Unknown");
  expect(markup).not.toContain(">Connected<");
  expect(markup).not.toContain(">Revoked<");
  expect(markup).toContain("Saved team endpoint");
  expect(markup).toContain("Default bookmark");
  expect(markup).toContain("Manage endpoints");
});

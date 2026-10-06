import assert from "node:assert/strict";
import { test } from "node:test";
import {
  AgentRunServiceService,
  ListVisibleRunsRequest,
  SetWorkspaceSharingRequest,
  VisibleRun,
} from "../src/gen/colossus/api/v1alpha1/agent_run.js";

test("missing disclosure authority remains read-only on the public wire", () => {
  const legacy = VisibleRun.decode(new Uint8Array());
  assert.equal(legacy.controllable, false);
  assert.equal(legacy.continuable, false);
  const access = VisibleRun.fromPartial({
    controllable: false,
    continuable: true,
  });
  assert.deepEqual(
    VisibleRun.decode(VisibleRun.encode(access).finish()),
    access,
  );
  const request = SetWorkspaceSharingRequest.fromPartial({
    recipientApplicationId: "app:cloud",
    enabled: true,
    allowContinuation: false,
  });
  assert.deepEqual(
    SetWorkspaceSharingRequest.decode(
      SetWorkspaceSharingRequest.encode(request).finish(),
    ),
    request,
  );
  assert.equal("ownerApplicationId" in request, false);
});

test("discovery and sharing use dedicated bounded public RPC shapes", () => {
  assert.equal(
    AgentRunServiceService.listVisibleRuns.path,
    "/colossus.api.v1alpha1.AgentRunService/ListVisibleRuns",
  );
  assert.equal(
    AgentRunServiceService.setWorkspaceSharing.path,
    "/colossus.api.v1alpha1.AgentRunService/SetWorkspaceSharing",
  );
  const request = ListVisibleRunsRequest.fromPartial({
    page: { pageSize: 32, pageToken: "source-cursor" },
    includeArchived: true,
  });
  assert.equal(
    ListVisibleRunsRequest.decode(
      ListVisibleRunsRequest.encode(request).finish(),
    ).page?.pageToken,
    "source-cursor",
  );
  assert.equal("ownerApplicationId" in request, false);
});

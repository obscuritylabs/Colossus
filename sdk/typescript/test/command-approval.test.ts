import assert from "node:assert/strict";
import { test } from "node:test";
import { ApprovalInteraction } from "../src/gen/colossus/api/v1alpha1/agent_run.js";

test("command context round trips without collapsing argv and legacy absence remains absent", () => {
  const legacy = ApprovalInteraction.fromPartial({
    reason: "Approval required",
    action: "process.execute",
    resource: "configured executable",
    requestHash: "binding",
  });
  assert.equal(
    ApprovalInteraction.decode(ApprovalInteraction.encode(legacy).finish())
      .commandContext,
    undefined,
  );
  const approval = ApprovalInteraction.fromPartial({
    ...legacy,
    commandContext: {
      justification: "Check dependency versions.",
      executable: "/bin/sh",
      arguments: ["-c", "echo 'two  spaces'", "", "é"],
      workingDirectory: "/work/project",
      redacted: true,
    },
  });
  assert.deepEqual(
    ApprovalInteraction.decode(ApprovalInteraction.encode(approval).finish()),
    approval,
  );
});

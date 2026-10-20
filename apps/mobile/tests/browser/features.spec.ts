import { expect, test, type Page } from "@playwright/test";

async function fixture(page: Page, source = "") {
  await page.route("**/src/preview.ts*", async (route) => {
    const response = await route.fetch();
    await route.fulfill({
      response,
      body: (await response.text()) + `\n${source}`,
    });
  });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Explore development preview" })
    .click();
}
const capture = `
const featureRequest = previewClient.request.bind(previewClient);
const featureAttempts = [];
Object.defineProperty(globalThis,"__featureAttempts",{value:featureAttempts});
previewClient.request = async (operation) => {
  featureAttempts.push(structuredClone(operation));
  if(operation.kind === "runtime_context") return {connection_id:operation.connection_id,kind:"result",value:{capabilities:["runtime.model_catalog.v1","agent_runs.model_selection","research.create","goal.create","agent_messages.read.v1"]}};
  if(operation.kind === "model_catalog") return {connection_id:operation.connection_id,kind:"result",value:{model_profiles:[{profile:"fast",model:"Fast test model",provider:"Test provider",provider_profile:"test",reasoning_effort:"medium",default_roles:["primary"]},{profile:"careful",model:"Careful test model",provider:"Test provider",provider_profile:"test",reasoning_effort:"high",default_roles:[]}]}};
  return featureRequest(operation);
};
nodes.forEach((entry) => {entry.node.policy = {...entry.node.policy,models:entry.node.policy?.models ?? [],approval_mode:entry.node.policy?.approval_mode ?? "always",allowed_tools:["filesystem.search","web.search"]};});
`;

test("model, reasoning, Goal and Research settings are captured in submitted requests", async ({
  page,
}) => {
  await fixture(page, capture);
  await page.getByRole("button", { name: "New task", exact: true }).click();
  await page.getByRole("button", { name: "Task options · Execute" }).click();
  await page
    .getByRole("button", { name: /Choose model and reasoning:/ })
    .click();
  await page.getByRole("option", { name: /Careful test model/ }).click();
  await page
    .getByRole("slider", { name: "Reasoning effort for next message" })
    .press("End");
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("dialog", { name: "Task options", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Choose model and reasoning:/ }),
  ).toBeFocused();
  await page.locator(".mode-switch label").filter({ hasText: "Goal" }).click();
  await page.getByRole("spinbutton", { name: "Goal iterations" }).fill("7");
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Message Colossus" })
    .fill("Complete the goal");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Complete the goal", exact: true }),
  ).toBeVisible();
  const goal = await page.evaluate(() =>
    Reflect.get(globalThis, "__featureAttempts").find(
      (operation: { kind: string }) => operation.kind === "create_thread",
    ),
  );
  expect(goal.request).toMatchObject({
    mode: "goal",
    goal_max_iterations: 7,
    model_profile: "careful",
    reasoning_effort: "ultra",
  });
  await page.getByRole("button", { name: "Task options · Goal" }).click();
  await page
    .locator(".mode-switch label")
    .filter({ hasText: "Research" })
    .click();
  await page
    .locator(".research-depth-option")
    .filter({ hasText: "Deep" })
    .click();
  await page.getByText("Search the public web", { exact: true }).click();
  await expect(
    page
      .locator(".research-source-option")
      .filter({ hasText: "MCP connections" })
      .getByRole("checkbox"),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Message Colossus" })
    .fill("Research the evidence");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          Reflect.get(globalThis, "__featureAttempts").filter(
            (operation: { kind: string }) => operation.kind === "send_message",
          ).length,
      ),
    )
    .toBe(1);
  const research = await page.evaluate(() =>
    Reflect.get(globalThis, "__featureAttempts").find(
      (operation: { kind: string }) => operation.kind === "send_message",
    ),
  );
  expect(research.request).toMatchObject({
    mode: "research",
    research_depth: "deep",
    research_sources: ["repo", "web"],
  });
  expect(research.request).not.toHaveProperty("model_profile");
  expect(research.request).not.toHaveProperty("reasoning_effort");
  await page
    .getByRole("button", { name: "Task options · Research", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: /Choose model and reasoning:/ }),
  ).toHaveCount(0);
  await expect(
    page.getByText(
      "Research uses the model configured for the host’s research worker.",
      { exact: true },
    ),
  ).toBeVisible();
  await page
    .locator(".mode-switch label")
    .filter({ hasText: "Execute" })
    .click();
  await expect(
    page.getByRole("button", {
      name: /Choose model and reasoning: Careful test model/,
    }),
  ).toContainText("ultra");
  await page.getByRole("button", { name: "Done", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Message Colossus" })
    .fill("Return to execution");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          Reflect.get(globalThis, "__featureAttempts").filter(
            (operation: { kind: string }) => operation.kind === "send_message",
          ).length,
      ),
    )
    .toBe(2);
  const resumed = await page.evaluate(() =>
    Reflect.get(globalThis, "__featureAttempts")
      .filter(
        (operation: { kind: string }) => operation.kind === "send_message",
      )
      .at(-1),
  );
  expect(resumed.request).toMatchObject({
    mode: "execute",
    model_profile: "careful",
    reasoning_effort: "ultra",
  });
});

test("unsupported connections keep model selection and advanced modes unavailable", async ({
  page,
}) => {
  await fixture(
    page,
    capture +
      `
const supportedRequest = previewClient.request.bind(previewClient);
previewClient.request = async (operation) => operation.kind === "runtime_context" ? {connection_id:operation.connection_id,kind:"result",value:{capabilities:[]}} : supportedRequest(operation);
`,
  );
  await page.getByRole("button", { name: "New task", exact: true }).click();
  await page.getByRole("button", { name: "Task options · Execute" }).click();
  await expect(
    page.getByRole("radio", { name: "Goal", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("radio", { name: "Research", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: /Choose model and reasoning:/ }),
  ).toHaveCount(0);
  await expect(page.getByText(/Model selection is unavailable/)).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        Reflect.get(globalThis, "__featureAttempts").filter(
          (operation: { kind: string }) => operation.kind === "model_catalog",
        ).length,
    ),
  ).toBe(0);
});

test("conversation streaming shows released output, tool lifecycle and reasoning with exact run binding", async ({
  page,
}) => {
  await fixture(
    page,
    `
messages.set("thread-0",messages.get("thread-0").filter((message) => message.role === "user"));
previewClient.watch = async (scope,listener) => {
  if(scope.thread === "thread-0") Object.defineProperty(globalThis,"__emitMobile",{value:listener,configurable:true});
  listener({kind:"connection",connected:true});
  return async () => {};
};
`,
  );
  await page
    .getByRole("button", { name: /Investigate connection timeout/ })
    .click();
  await expect(page.getByText("Live activity", { exact: true })).toBeVisible();
  await page.evaluate(() => {
    const emit = Reflect.get(globalThis, "__emitMobile");
    const update = (
      sequence: number,
      body: Record<string, unknown>,
      run_id = "run-0",
    ) =>
      emit({
        kind: "run_update",
        cursor: String(sequence),
        update: {
          task_id: "task-0",
          run_id,
          sequence,
          created_at: new Date().toISOString(),
          update: body,
        },
      });
    update(1, { output_delta: "Fresh live response" });
    update(2, { output_delta: "Foreign run must stay hidden" }, "run-foreign");
    update(3, {
      tool_activity: {
        call_id: "call-1",
        tool_name: "filesystem.read",
        state: "started",
        summary: "Reading the file",
        input: "src/example.rs",
      },
    });
    update(4, {
      tool_activity: {
        call_id: "call-1",
        tool_name: "filesystem.read",
        state: "completed",
        summary: "File read",
        preview: "Released preview text",
      },
    });
    update(5, { reasoning_summary: "Released reasoning summary" });
  });
  await expect(
    page.getByText("Fresh live response", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText(/Foreign run must stay hidden/)).toHaveCount(0);
  await expect(
    page.getByText("Released reasoning summary", { exact: true }),
  ).toBeVisible();
  await page.locator(".shared-activity-tool summary").click();
  await expect(page.getByText("src/example.rs", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Released preview text", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Reading the file", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("File read", { exact: true })).toBeVisible();
});

test("uncertain interaction responses preserve the original request and replay identities", async ({
  page,
}) => {
  await fixture(
    page,
    `
const responseRequest = previewClient.request.bind(previewClient);
const responseAttempts = [];
Object.defineProperty(globalThis,"__responseAttempts",{value:responseAttempts});
previewClient.request = async (operation) => {
  if(operation.kind !== "respond") return responseRequest(operation);
  responseAttempts.push(structuredClone(operation));
  if(responseAttempts.length === 1) {
    tasks[1].snapshot.pending_interactions[0].etag = "changed-etag";
    throw new ClientFailure("outcome_unknown","The response could not be confirmed.");
  }
  if(responseAttempts.length === 2) throw new ClientFailure("permission_denied","Reconciliation is temporarily unavailable.");
  tasks[1].snapshot.pending_interactions[0].status = "answered";
  return {command:{command_id:"confirmed-response",reply:{kind:"result"}}};
};
tasks[1].snapshot.pending_interactions[0].content.approval.command_context = {justification:"Prepared action",executable:"cargo",arguments:["test","--locked"],working_directory:"/workspace/Colossus",redacted:false};
`,
  );
  await page.getByRole("button", { name: /Review cancellation flow/ }).click();
  await expect(page.getByText("cargo", { exact: true })).toBeVisible();
  await expect(page.getByText('"--locked"', { exact: true })).toBeVisible();
  await expect(
    page.getByText("/workspace/Colossus", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Approve this action", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText("could not be confirmed");
  await expect(
    page.getByRole("button", { name: "Deny", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Settings", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("button", { name: "Reconcile original response", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText(
    "temporarily unavailable",
  );
  await page
    .getByRole("button", { name: "Reconcile original response", exact: true })
    .click();
  await expect(
    page.getByText("Response confirmed by the runtime.", { exact: true }),
  ).toBeVisible();
  const attempts = await page.evaluate(() =>
    Reflect.get(globalThis, "__responseAttempts"),
  );
  expect(attempts).toHaveLength(3);
  expect(attempts[1]).toEqual(attempts[0]);
  expect(attempts[2]).toEqual(attempts[0]);
});

test("earlier message and task pages load locally and remain after refresh", async ({
  page,
}) => {
  await fixture(
    page,
    `
const historyRequest = previewClient.request.bind(previewClient);
previewClient.request = async (operation) => {
  const result = await historyRequest(operation);
  if(operation.kind !== "thread" || operation.thread !== "thread-2") return result;
  if(operation.message_after === "older-messages" && operation.task_after === "older-tasks") return {...result,messages:[{message_id:"earlier-message",role:"system",text:"Earlier retained history",created_at:"2025-01-01T00:00:00Z"}],tasks:[],next_message_cursor:"even-older",next_task_cursor:null};
  if(operation.message_after === "even-older" && !operation.task_after) return {...result,messages:[{message_id:"oldest-message",role:"system",text:"Oldest retained history",created_at:"2024-01-01T00:00:00Z"}],tasks:[],next_message_cursor:null,next_task_cursor:"older-tasks"};
  return {...result,next_message_cursor:"older-messages",next_task_cursor:"older-tasks"};
};
`,
  );
  await page.getByRole("button", { name: /Prepare release notes/ }).click();
  await page
    .getByRole("button", { name: "Load earlier history", exact: true })
    .click();
  await expect(
    page.getByText("Earlier retained history", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Refresh conversation", exact: true })
    .click();
  await expect(
    page.getByText("Earlier retained history", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Load earlier history", exact: true })
    .click();
  await expect(
    page.getByText("Oldest retained history", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Load earlier history", exact: true }),
  ).toHaveCount(0);
});

test("session views expose released activity, participants, plans, inboxes and resources", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await fixture(page);
  await page.getByRole("button", { name: /Prepare release notes/ }).click();
  await page
    .getByRole("button", { name: "Session views", exact: true })
    .click();
  for (const name of [
    "Activity",
    "Participants",
    "Plans",
    "Agent inboxes",
    "Sources",
    "Resources",
    "Snapshots",
  ]) {
    await page
      .getByRole("combobox", { name: "Session view", exact: true })
      .click();
    await page.getByRole("option", { name, exact: true }).click();
    await expect(
      page.getByRole("region", { name: "Session details", exact: true }),
    ).toBeVisible();
    if (name === "Participants") {
      const participants = page.getByRole("region", {
        name: "Session participants",
        exact: true,
      });
      await expect(participants.getByRole("listitem")).toHaveCount(1);
      await expect(
        participants.getByText("Prepare release notes", { exact: true }),
      ).toBeVisible();
    }
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  }
  expect(errors).toEqual([]);
});

test("a late capability reply cannot enable controls for the newly selected workspace", async ({
  page,
}) => {
  await fixture(
    page,
    capture +
      `
const currentRequest = previewClient.request.bind(previewClient);
previewClient.request = async (operation) => {
  if(operation.kind === "runtime_context" && operation.node === "node-0") return new Promise(resolve => {Object.defineProperty(globalThis,"__resolveOldContext",{value:() => resolve({kind:"result",connection_id:operation.connection_id,value:{capabilities:["runtime.model_catalog.v1","agent_runs.model_selection","goal.create","research.create"]}}),configurable:true});});
  if(operation.kind === "runtime_context") return {kind:"result",connection_id:operation.connection_id,value:{capabilities:[]}};
  return currentRequest(operation);
};
`,
  );
  await page.getByRole("button", { name: "New task", exact: true }).click();
  await page.getByRole("button", { name: "Task options · Execute" }).click();
  await page
    .getByRole("radiogroup", { name: "Execution workspace", exact: true })
    .getByRole("radio")
    .nth(1)
    .click();
  await page.evaluate(() => Reflect.get(globalThis, "__resolveOldContext")());
  await expect(
    page.getByRole("radio", { name: "Goal", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("radio", { name: "Research", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: /Choose model and reasoning:/ }),
  ).toHaveCount(0);
});

test("inline questions send exact choices and free-form answers", async ({
  page,
}) => {
  await fixture(
    page,
    capture +
      `
const question = {run_id:"run-1",kind:"user_prompt",status:"pending",respondable_by_caller:true,etag:"question-etag",content:{user_prompt:{question:"Choose a test target",choices:[{choice_id:"mac",label:"This Mac"}],allow_free_form:false}}};
tasks[1].snapshot.pending_interactions = [{...question,interaction_id:"question-choice"},{...question,interaction_id:"question-free",content:{user_prompt:{question:"Describe the expected result",choices:[],allow_free_form:true}}}];
`,
  );
  await page.getByRole("button", { name: /Review cancellation flow/ }).click();
  await page.getByRole("button", { name: "This Mac", exact: true }).click();
  await expect(
    page.getByText("Response confirmed by the runtime.", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("textbox", { name: "Your answer", exact: true })
    .fill("Keep the native session connected");
  await page.getByRole("button", { name: "Send answer", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          Reflect.get(globalThis, "__featureAttempts").filter(
            (operation: { kind: string }) => operation.kind === "respond",
          ).length,
      ),
    )
    .toBe(2);
  const attempts = await page.evaluate(() =>
    Reflect.get(globalThis, "__featureAttempts").filter(
      (operation: { kind: string }) => operation.kind === "respond",
    ),
  );
  expect(attempts[0].request.response).toEqual({
    prompt: { choice: { choice_id: "mac", label: "This Mac" } },
  });
  expect(attempts[1].request.response).toEqual({
    prompt: { free_form: "Keep the native session connected" },
  });
});

test("shared read-only runs cannot approve runtime actions", async ({
  page,
}) => {
  await fixture(page, capture + `tasks[1].source_read_only = true;`);
  await page.getByRole("button", { name: /Review cancellation flow/ }).click();
  await expect(
    page.getByRole("button", { name: "Approve this action", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Deny", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(
      () =>
        Reflect.get(globalThis, "__featureAttempts").filter(
          (operation: { kind: string }) => operation.kind === "respond",
        ).length,
    ),
  ).toBe(0);
});

test("Inbox review opens the in-app request and returns to Work", async ({
  page,
}) => {
  await fixture(
    page,
    `
const olderPending = structuredClone(tasks[1]);
olderPending.task_id = "older-pending-task";
olderPending.run_id = "older-pending-run";
olderPending.request.input = [{text:"Review an older request"}];
olderPending.snapshot.run.run_id = olderPending.run_id;
olderPending.snapshot.run.title = "Review an older request";
olderPending.snapshot.pending_interactions[0].run_id = olderPending.run_id;
olderPending.snapshot.pending_interactions[0].interaction_id = "older-pending-interaction";
tasks.push(olderPending);
const pagedInboxRequest = previewClient.request.bind(previewClient);
previewClient.request = async (operation) => {
  if(operation.kind === "tasks") return operation.after === "older-pending-page" ? {tasks:[olderPending],next_cursor:null} : {tasks:tasks.filter(task => task.task_id !== olderPending.task_id),next_cursor:"older-pending-page"};
  return pagedInboxRequest(operation);
};
`,
  );
  await page.getByRole("button", { name: /Prepare release notes/ }).click();
  await page.getByRole("button", { name: "Inbox", exact: true }).click();
  await page
    .getByRole("button", { name: "Review request", exact: true })
    .click();
  await expect(
    page.getByRole("heading", {
      name: /Review the cancellation flow/,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Approve this action", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Approve this action", exact: true }),
  ).toBeEnabled();
  await page
    .getByRole("button", { name: "Back to tasks", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Recent tasks", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Conversations", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Recent work", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: /Prepare release notes/ }).click();
  await page.getByRole("button", { name: "Inbox", exact: true }).click();
  await page
    .getByRole("button", { name: "Load older requests", exact: true })
    .click();
  const older = page.locator(".mobile-attention").filter({
    has: page.getByRole("heading", {
      name: "Review an older request",
      exact: true,
    }),
  });
  await older
    .getByRole("button", { name: "Review request", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Review an older request", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Approve this action", exact: true }),
  ).toBeEnabled();
  await page
    .getByRole("button", { name: "Approve this action", exact: true })
    .click();
  await expect(
    page.getByText("Response confirmed by the runtime.", { exact: true }),
  ).toBeVisible();

  await page
    .getByRole("button", { name: "Back to tasks", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Recent tasks", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Inbox", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Review an older request", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Review request", exact: true }),
  ).toHaveCount(1);
});

test("a failed status refresh preserves the confirmed response and disables its old action", async ({
  page,
}) => {
  await fixture(
    page,
    `
const knownReceiptRequest = previewClient.request.bind(previewClient);
let receiptConfirmed = false;
const knownReceiptAttempts = [];
Object.defineProperty(globalThis,"__knownReceiptAttempts",{value:knownReceiptAttempts});
previewClient.request = async (operation) => {
  if(operation.kind === "respond") {
    knownReceiptAttempts.push(structuredClone(operation));
    const reply = await knownReceiptRequest(operation);
    receiptConfirmed = true;
    return reply;
  }
  if(receiptConfirmed && operation.kind === "task" && operation.task === "task-1") throw new ClientFailure("unavailable","Task status is temporarily unavailable.");
  return knownReceiptRequest(operation);
};
`,
  );
  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await page.getByRole("button", { name: /Review.*cancellation flow/ }).click();
  await page
    .getByRole("button", { name: "Approve this action", exact: true })
    .click();
  await expect(
    page.getByText("Response confirmed by the runtime.", { exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("alert")).toContainText(
    "temporarily unavailable",
  );
  await expect(
    page.getByRole("button", { name: "Approve this action", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", {
      name: "Reconcile original response",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Back to tasks", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(
      () => Reflect.get(globalThis, "__knownReceiptAttempts").length,
    ),
  ).toBe(1);
});

// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RunComposer, type RunRequest } from "./RunComposer";
import type { FleetNode } from "./api";

const caps = [
  "runtime.model_catalog.v1",
  "agent_runs.model_selection",
  "research.create",
  "goal.create",
];
const profiles = [
  {
    profile: "fast",
    model: "Fast model",
    provider: "Test provider",
    provider_profile: "test",
    reasoning_effort: "medium",
    default_roles: ["reviewer"],
  },
  {
    profile: "careful",
    model: "Careful model",
    provider: "Test provider",
    provider_profile: "test",
    reasoning_effort: "high",
    default_roles: ["primary"],
  },
];
function node(connection = "a", project = "project", id = "node"): FleetNode {
  return {
    node: {
      node_id: id,
      project_id: project,
      instance_id: id,
      label: "Workspace",
      certificate_sha256: "fixture",
      roles: ["primary", "reviewer"],
      revoked: false,
      revision: 1,
      policy: {
        schema_version: 1,
        provenance: "runtime",
        fingerprint: "fixture",
        configuration_revision: 1,
        access_profile: "fixture",
        sandbox_backend: "fixture",
        sandbox_profile: "fixture",
        boundary_acknowledged: true,
        approval_mode: "always",
        allowed_roles: ["primary", "reviewer"],
        allowed_tools: ["filesystem.search"],
        capabilities: [],
        models: [],
        findings: [],
        telemetry: {
          provenance: "runtime",
          denied_requests: null,
          approval_requests: null,
          outcome_unknown_runs: null,
        },
      },
    },
    presence: { ready: true, connection_id: connection, capabilities: caps },
  };
}
interface ResourceCall {
  path: string;
  connection: string;
  operation: string;
  signal: AbortSignal;
}
const response = (connection: string, value: unknown, kind = "result") =>
  new Response(JSON.stringify({ connection_id: connection, kind, value }));
let root: Root, container: HTMLDivElement;
let calls: ResourceCall[];
let resolveResource: (call: ResourceCall) => Response | Promise<Response>;
const onSubmit = vi.fn<(request: RunRequest) => Promise<boolean>>();
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.defineProperty(document, "fonts", {
    configurable: true,
    value: new EventTarget(),
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  calls = [];
  onSubmit.mockReset().mockResolvedValue(false);
  resolveResource = ({ connection, operation }) =>
    response(
      connection,
      operation === "context"
        ? { capabilities: caps }
        : { model_profiles: profiles },
    );
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string, options: RequestInit) => {
      const body = JSON.parse(String(options.body));
      const call = {
        path,
        connection: body.connection_id,
        operation: body.operation.operation,
        signal: options.signal as AbortSignal,
      };
      calls.push(call);
      return resolveResource(call);
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function render(target = node(), disabled = false) {
  await act(async () =>
    root.render(
      <RunComposer
        nodes={[target]}
        nodeId={target.node.node_id}
        lockedRuntime
        disabled={disabled}
        busy={false}
        label="Message"
        action="Send"
        onSubmit={onSubmit}
      />,
    ),
  );
}
async function draft(text = "Keep this request") {
  const input = container.querySelector<HTMLTextAreaElement>("textarea")!;
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )!.set!.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function send() {
  await act(async () =>
    container
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
}
function picker() {
  return container.querySelector<HTMLButtonElement>(
    '[aria-label^="Choose model and reasoning:"]',
  );
}
async function choose(label: string, effort = 0) {
  expect(picker()).not.toBeNull();
  await act(() => picker()!.click());
  const option = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((element) => element.querySelector("strong")?.textContent === label)!;
  expect(option).toBeDefined();
  await act(() => option.click());
  const slider = document.querySelector<HTMLElement>('[role="slider"]')!;
  await act(() =>
    slider.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Home", bubbles: true }),
    ),
  );
  for (let index = 0; index < effort; index++)
    await act(() =>
      slider.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
      ),
    );
  const done = [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => button.textContent === "Done",
  )!;
  await act(() => done.click());
}
async function mode(value: string) {
  await act(() =>
    container
      .querySelector<HTMLInputElement>(`input[value="${value}"]`)!
      .click(),
  );
}

it("keeps role/model defaults unset and binds deliberate model or effort edits to new retry identities", async () => {
  await render();
  expect(calls.map((call) => call.operation)).toEqual([
    "context",
    "model_catalog",
  ]);
  expect(picker()!.textContent).toContain("Careful model");
  expect(picker()!.title).toContain("Model default · high");
  await draft();
  await send();
  await send();
  const first = onSubmit.mock.calls[0]![0];
  expect(first).not.toHaveProperty("model_profile");
  expect(first).not.toHaveProperty("reasoning_effort");
  expect(onSubmit.mock.calls[1]![0]).toBe(first);
  await choose("Fast model", 3);
  await send();
  const selected = onSubmit.mock.calls[2]![0];
  expect(selected).toMatchObject({
    model_profile: "fast",
    reasoning_effort: "low",
  });
  expect(selected.idempotency_key).not.toBe(first.idempotency_key);
  await choose("Fast model", 5);
  await send();
  expect(onSubmit.mock.calls[3]![0].reasoning_effort).toBe("high");
  expect(onSubmit.mock.calls[3]![0].idempotency_key).not.toBe(
    selected.idempotency_key,
  );
  await choose("Role default");
  await send();
  expect(onSubmit.mock.calls[4]![0]).not.toHaveProperty("model_profile");
  expect(onSubmit.mock.calls[4]![0]).not.toHaveProperty("reasoning_effort");
});

it("uses configured choices in Execute/Plan/Goal and preserves them while Research uses its own route", async () => {
  const target = node();
  await render(target);
  await draft();
  await choose("Fast model", 3);
  for (const value of ["execute", "plan", "goal"]) {
    await mode(value);
    await send();
    expect(onSubmit.mock.lastCall![0]).toMatchObject({
      mode: value,
      model_profile: "fast",
      reasoning_effort: "low",
    });
  }
  await mode("research");
  expect(picker()).toBeNull();
  expect(container.textContent).toContain("Host research model");
  await send();
  expect(onSubmit.mock.lastCall![0].mode).toBe("research");
  expect(onSubmit.mock.lastCall![0]).not.toHaveProperty("model_profile");
  expect(onSubmit.mock.lastCall![0]).not.toHaveProperty("reasoning_effort");
  await mode("execute");
  expect(picker()!.textContent).toContain("Fast model");
  await send();
  expect(onSubmit.mock.lastCall![0]).toMatchObject({
    model_profile: "fast",
    reasoning_effort: "low",
  });
});

it.each(["older", "missing-selection", "read-only", "offline", "revoked"])(
  "does not request a model catalog for %s targets",
  async (condition) => {
    const target = node();
    if (condition === "older")
      target.presence!.capabilities = ["research.create", "goal.create"];
    if (condition === "missing-selection")
      target.presence!.capabilities = caps.filter(
        (capability) => capability !== "agent_runs.model_selection",
      );
    if (condition === "offline") target.presence!.ready = false;
    if (condition === "revoked") target.node.revoked = true;
    await render(target, condition === "read-only");
    expect(calls).toEqual([]);
    expect(picker()).toBeNull();
  },
);

it.each([
  "wrong-context",
  "missing-context-capability",
  "wrong-catalog",
  "empty",
  "malformed",
])("fails closed for %s catalog replies", async (condition) => {
  resolveResource = ({ operation, connection }) => {
    if (operation === "context")
      return response(condition === "wrong-context" ? "other" : connection, {
        capabilities:
          condition === "missing-context-capability" ? ["goal.create"] : caps,
      });
    return response(condition === "wrong-catalog" ? "other" : connection, {
      model_profiles:
        condition === "empty"
          ? []
          : condition === "malformed"
            ? [{ ...profiles[0], reasoning_effort: "invented" }]
            : profiles,
    });
  };
  await render();
  expect(picker()).toBeNull();
  expect(calls.map((call) => call.operation)).toEqual(
    condition === "wrong-context" || condition === "missing-context-capability"
      ? ["context"]
      : ["context", "model_catalog"],
  );
  await draft();
  await send();
  expect(onSubmit.mock.lastCall![0]).not.toHaveProperty("model_profile");
});

it.each(["context", "model_catalog"])(
  "ignores a delayed %s from an old project/node/connection and aborts it",
  async (deferredOperation) => {
    let release!: (response: Response) => void;
    resolveResource = ({ operation, connection }) => {
      if (connection === "a" && operation === deferredOperation)
        return new Promise((resolve) => {
          release = resolve;
        });
      return response(
        connection,
        operation === "context"
          ? { capabilities: caps }
          : {
              model_profiles: [
                {
                  ...profiles[0],
                  model: "Current model",
                  default_roles: ["primary"],
                },
              ],
            },
      );
    };
    await render();
    const stale = calls.at(-1)!;
    await render(node("b", "other-project", "other-node"));
    expect(stale.signal.aborted).toBe(true);
    expect(picker()!.textContent).toContain("Current model");
    await act(() =>
      release(
        response(
          "a",
          deferredOperation === "context"
            ? { capabilities: caps }
            : { model_profiles: profiles },
        ),
      ),
    );
    expect(picker()!.textContent).toContain("Current model");
    expect(
      calls.filter(
        (call) => call.connection === "a" && call.operation === "model_catalog",
      ),
    ).toHaveLength(deferredOperation === "context" ? 0 : 1);
    expect(calls.at(-1)!.path).toBe(
      "/api/projects/other-project/nodes/other-node/resources",
    );
  },
);

it("reconciles an unchanged failed packet after reconnecting, while a deliberate target change gets a new identity", async () => {
  await render();
  await draft();
  await choose("Fast model", 3);
  await send();
  const original = structuredClone(onSubmit.mock.lastCall![0]);
  await render(node("replacement"));
  expect(picker()!.textContent).toContain("Careful model");
  expect(container.textContent).toContain("reconciles its original settings");
  await send();
  expect(onSubmit.mock.lastCall![0]).toEqual(original);
  // Choosing the displayed role default is still an explicit change from the
  // retained override, even though reconnecting already cleared the picker.
  await choose("Role default");
  await send();
  const defaultChoice = onSubmit.mock.lastCall![0];
  expect(defaultChoice).not.toHaveProperty("model_profile");
  expect(defaultChoice).not.toHaveProperty("reasoning_effort");
  expect(defaultChoice.idempotency_key).not.toBe(original.idempotency_key);
  await render(node("replacement", "project", "another-node"));
  await send();
  const changed = onSubmit.mock.lastCall![0];
  expect(changed.idempotency_key).not.toBe(original.idempotency_key);
  expect(changed.idempotency_key).not.toBe(defaultChoice.idempotency_key);
  expect(changed).not.toHaveProperty("model_profile");
  expect(changed).not.toHaveProperty("reasoning_effort");
});

it("retains the reviewed Research role and sources when fleet updates change the displayed defaults and tool ceiling", async () => {
  const originalTarget = node();
  await render(originalTarget);
  await draft();
  await mode("research");
  await send();
  const original = structuredClone(onSubmit.mock.lastCall![0]);
  expect(original).toMatchObject({
    role: "primary",
    research_sources: ["repo"],
  });
  const replacement = node("replacement");
  replacement.node.roles = ["reviewer"];
  replacement.node.policy!.allowed_tools = [];
  await render(replacement);
  expect(container.textContent).toContain("reconciles its original settings");
  await send();
  expect(onSubmit.mock.lastCall![0]).toEqual(original);
  await draft("A deliberately revised Research request");
  await send();
  // Editing the human request creates new intent; its now-unauthorized source
  // must block a fresh submission rather than silently reuse the prior packet.
  expect(onSubmit).toHaveBeenCalledTimes(2);
});

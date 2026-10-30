// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { RunComposer, type RunRequest } from "./RunComposer";
import type { FleetNode } from "./api";
import { NextPromptModeContext } from "./Appearance";

it("keeps Model mode local until the connected runtime advertises prediction", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.defineProperty(document, "fonts", {configurable:true,value:new EventTarget()});
  vi.stubGlobal("fetch", vi.fn(async () => reply([])));
  const container = document.createElement("div");
  const root = createRoot(container);
  const load = vi.fn(async () => null);
  const render = async (target: FleetNode) => act(async () => root.render(
    <NextPromptModeContext value="model"><RunComposer nodes={[target]} nodeId={target.node.node_id} busy={false} label="Message" action="Send" onSubmit={async () => true} nextPromptSource={{key:"source",output:"I can review the deployment."}} loadPromptPrediction={load} /></NextPromptModeContext>,
  ));
  const unsupported = node("unsupported");
  await render(unsupported);
  expect(load).not.toHaveBeenCalled();
  expect(container.textContent).toContain("Review the deployment");
  await render({...unsupported,presence:{...unsupported.presence!,ready:true,capabilities:["composer.prediction.v1"]}});
  expect(load).toHaveBeenCalledTimes(1);
  await act(async () => root.unmount());
  vi.unstubAllGlobals();
});

const node = (id: string): FleetNode => ({
  node: {
    node_id: id,
    project_id: "project",
    instance_id: id,
    label: "Workspace",
    certificate_sha256: "fixture",
    roles: ["primary"],
    revoked: false,
    revision: 1,
    policy: {
      schema_version: 1,
      provenance: "runtime",
      fingerprint: "fixture",
      configuration_revision: 1,
      access_profile: "test",
      sandbox_backend: "test",
      sandbox_profile: "test",
      boundary_acknowledged: true,
      approval_mode: "risk_auto",
      allowed_roles: ["primary"],
      allowed_tools: ["filesystem.search", "web.search"],
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
  presence: {
    ready: true,
    connection_id: `connection-${id}`,
    capabilities: ["runtime.resources.v1"],
  },
});
const reply = (capabilities: string[], connection = "connection-one") =>
  new Response(
    JSON.stringify({
      connection_id: connection,
      kind: "result",
      value: { capabilities },
    }),
  );
async function fixture(target = node("one"), hostedProject?: string) {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.defineProperty(document, "fonts", {
    configurable: true,
    value: new EventTarget(),
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onSubmit = vi.fn<(input: RunRequest) => Promise<boolean>>(
    async () => true,
  );
  const render = async (target: FleetNode) => {
    await act(async () =>
      root.render(
        <RunComposer
          nodes={[target]}
          nodeId={target.node.node_id}
          lockedRuntime
          hostedProject={hostedProject}
          busy={false}
          label="Message"
          action="Send"
          onSubmit={onSubmit}
        />,
      ),
    );
  };
  await render(target);
  return {
    container,
    onSubmit,
    render,
    close: async () => {
      await act(() => root.unmount());
      container.remove();
      vi.unstubAllGlobals();
    },
  };
}
it("sends explicit bounded Research settings through the existing run contract", async () => {
  const fetch = vi.fn(async (_path: string, _options: RequestInit) =>
    reply(["research.create"]),
  );
  vi.stubGlobal("fetch", fetch);
  const f = await fixture();
  try {
    expect(fetch.mock.calls[0]![0]).toBe(
      "/api/projects/project/nodes/one/resources",
    );
    expect(JSON.parse(String(fetch.mock.calls[0]![1].body)).connection_id).toBe(
      "connection-one",
    );
    await act(() =>
      f.container
        .querySelector<HTMLInputElement>('input[value="research"]')!
        .click(),
    );
    await act(() =>
      f.container
        .querySelector<HTMLInputElement>(
          '.research-depth-option input[value="deep"]',
        )!
        .click(),
    );
    const web = [
      ...f.container.querySelectorAll<HTMLLabelElement>(
        ".research-source-option",
      ),
    ].find((label) => label.textContent?.includes("Search the public web"))!;
    await act(() => web.querySelector<HTMLInputElement>("input")!.click());
    const mcp = [
      ...f.container.querySelectorAll<HTMLLabelElement>(
        ".research-source-option",
      ),
    ].find((label) => label.textContent?.includes("MCP connections"))!;
    expect(mcp.querySelector<HTMLInputElement>("input")!.disabled).toBe(true);
    const input = f.container.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      )!.set!.call(input, "Compare workspace evidence");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      f.container
        .querySelector("form")!
        .dispatchEvent(
          new Event("submit", { bubbles: true, cancelable: true }),
        );
    });
    expect(f.onSubmit).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({
        mode: "research",
        research_depth: "deep",
        research_sources: ["repo", "web"],
        input: [{ text: "Compare workspace evidence" }],
      }),
    );
  } finally {
    await f.close();
  }
});
it("requires at least one authorized evidence source and never falls back to Execute", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => reply(["research.create"])),
  );
  const f = await fixture();
  try {
    await act(() =>
      f.container
        .querySelector<HTMLInputElement>('input[value="research"]')!
        .click(),
    );
    const input = f.container.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      )!.set!.call(input, "Valid research question");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(() =>
      f.container
        .querySelector<HTMLInputElement>(
          ".research-source-option input:checked",
        )!
        .click(),
    );
    await act(() => {
      f.container
        .querySelector("form")!
        .dispatchEvent(
          new Event("submit", { bubbles: true, cancelable: true }),
        );
    });
    expect(f.onSubmit).not.toHaveBeenCalled();
    expect(f.container.textContent).toContain(
      "at least one authorized evidence source",
    );
    expect(
      f.container.querySelector<HTMLInputElement>('input[value="research"]')!
        .checked,
    ).toBe(true);
  } finally {
    await f.close();
  }
});
it("discards capability replies from an earlier runtime connection", async () => {
  let resolve: (value: Response) => void = () => {};
  const fetch = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise<Response>((done) => {
          resolve = done;
        }),
    )
    .mockResolvedValueOnce(reply([]));
  vi.stubGlobal("fetch", fetch);
  const f = await fixture();
  try {
    await f.render(node("two"));
    await act(() => resolve(reply(["research.create", "goal.create"])));
    expect(fetch.mock.calls[0]![1].signal.aborted).toBe(true);
    expect(
      f.container.querySelector<HTMLInputElement>('input[value="research"]')!
        .disabled,
    ).toBe(true);
    expect(
      f.container.querySelector<HTMLInputElement>('input[value="goal"]')!
        .disabled,
    ).toBe(true);
  } finally {
    await f.close();
  }
});

it("starts Goal without a Plan and binds retries to the reviewed iteration budget", async () => {
  vi.stubGlobal(
    "fetch",
    vi
      .fn()
      .mockResolvedValueOnce(reply(["goal.create"]))
      .mockResolvedValueOnce(reply([])),
  );
  const f = await fixture();
  f.onSubmit.mockResolvedValue(false);
  try {
    await act(() =>
      f.container
        .querySelector<HTMLInputElement>('input[value="goal"]')!
        .click(),
    );
    const input = f.container.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      )!.set!.call(input, "Verify the outcome");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const send = async () =>
      act(async () => {
        f.container
          .querySelector("form")!
          .dispatchEvent(
            new Event("submit", { bubbles: true, cancelable: true }),
          );
      });
    await send();
    await send();
    const first = f.onSubmit.mock.calls[0]![0];
    expect(first).toMatchObject({
      mode: "goal",
      goal_max_iterations: 5,
      plan_action: null,
      research_depth: null,
      research_sources: [],
    });
    expect(f.onSubmit.mock.calls[1]![0].idempotency_key).toBe(
      first.idempotency_key,
    );
    const budget = f.container.querySelector<HTMLInputElement>(
      'input[type="number"]',
    )!;
    const changeBudget = async (value: string) =>
      act(() => {
        Object.getOwnPropertyDescriptor(
          HTMLInputElement.prototype,
          "value",
        )!.set!.call(budget, value);
        budget.dispatchEvent(new Event("input", { bubbles: true }));
      });
    await changeBudget("51");
    await send();
    expect(f.onSubmit).toHaveBeenCalledTimes(2);
    await changeBudget("7");
    await send();
    expect(f.onSubmit.mock.calls[2]![0].goal_max_iterations).toBe(7);
    expect(f.onSubmit.mock.calls[2]![0].idempotency_key).not.toBe(
      first.idempotency_key,
    );
    const unavailable = node("two");
    unavailable.presence!.capabilities = [];
    await f.render(unavailable);
    await send();
    expect(f.onSubmit).toHaveBeenCalledTimes(3);
    expect(
      f.container.querySelector<HTMLInputElement>('input[value="goal"]')!
        .checked,
    ).toBe(true);
    expect(f.container.textContent).toContain(
      "Goal requires an available runtime",
    );
  } finally {
    await f.close();
  }
});

it("probes a connected replica with no local advertisements and respects unsupported replies", async () => {
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(
      reply(["research.create", "goal.create"], "connection-replica"),
    )
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ kind: "unsupported" })),
    );
  vi.stubGlobal("fetch", fetch);
  const target = node("replica");
  target.presence!.capabilities = [];
  const f = await fixture(target);
  try {
    expect(fetch).toHaveBeenCalledTimes(1);
    for (const mode of ["research", "goal"]) {
      expect(
        f.container.querySelector<HTMLInputElement>(`input[value="${mode}"]`)!
          .disabled,
      ).toBe(false);
    }
    const older = node("older");
    older.presence!.capabilities = [];
    await f.render(older);
    expect(fetch).toHaveBeenCalledTimes(2);
    for (const mode of ["research", "goal"]) {
      expect(
        f.container.querySelector<HTMLInputElement>(`input[value="${mode}"]`)!
          .disabled,
      ).toBe(true);
    }
    const offline = node("offline");
    offline.presence!.capabilities = [];
    offline.presence!.ready = false;
    await f.render(offline);
    expect(fetch).toHaveBeenCalledTimes(2);
  } finally {
    await f.close();
  }
});

it.each(["unchanged", "newer", "edited back"])(
  "consumes the submitted snapshot while preserving newer composer content: %s",
  async (edit) => {
    const editWhilePending = edit !== "unchanged";
    vi.stubGlobal(
      "fetch",
      vi.fn(async (path: string, options: RequestInit) => {
        if (path.endsWith("/hosted/files")) {
          const { file_name } = JSON.parse(String(options.body));
          return new Response(
            JSON.stringify({
              artifact: { artifact_id: file_name, file_name, size_bytes: 5 },
            }),
          );
        }
        return reply([]);
      }),
    );
    const f = await fixture(node("one"), "project");
    try {
      const input = f.container.querySelector<HTMLTextAreaElement>("textarea")!;
      const write = async (text: string) =>
        act(() => {
          Object.getOwnPropertyDescriptor(
            HTMLTextAreaElement.prototype,
            "value",
          )!.set!.call(input, text);
          input.dispatchEvent(new Event("input", { bubbles: true }));
        });
      const attach = async (name: string) => {
        const fileInput =
          f.container.querySelector<HTMLInputElement>('input[type="file"]')!;
        const file = new File(["hello"], name, { type: "text/plain" });
        Object.defineProperty(file, "arrayBuffer", {
          value: async () => new TextEncoder().encode("hello").buffer,
        });
        Object.defineProperty(fileInput, "files", {
          configurable: true,
          value: [file],
        });
        await act(async () => {
          fileInput.dispatchEvent(new Event("change", { bubbles: true }));
        });
      };
      const send = async () =>
        act(async () => {
          f.container
            .querySelector("form")!
            .dispatchEvent(
              new Event("submit", { bubbles: true, cancelable: true }),
            );
        });
      await write("Submitted question");
      await attach("submitted.txt");
      let complete: (result: boolean) => void = () => {
        throw new Error("No submission");
      };
      f.onSubmit.mockImplementationOnce(
        () =>
          new Promise<boolean>((resolve) => {
            complete = resolve;
          }),
      );
      await send();
      expect(f.onSubmit).toHaveBeenCalledTimes(1);
      expect(f.onSubmit.mock.calls[0]![0].input).toEqual([
        { text: "Submitted question" },
        { artifact: "submitted.txt" },
      ]);
      if (editWhilePending) {
        await write("My next question");
        if (edit === "edited back") await write("Submitted question");
        await attach("next.txt");
      }
      await act(() => complete(true));
      expect(input.value).toBe(
        editWhilePending
          ? edit === "edited back"
            ? "Submitted question"
            : "My next question"
          : "",
      );
      expect(
        f.container.querySelector('button[aria-label="Remove submitted.txt"]'),
      ).toBeNull();
      if (editWhilePending) {
        expect(
          f.container.querySelector('button[aria-label="Remove next.txt"]'),
        ).not.toBeNull();
        await send();
        expect(f.onSubmit.mock.calls[1]![0].input).toEqual([
          {
            text:
              edit === "edited back"
                ? "Submitted question"
                : "My next question",
          },
          { artifact: "next.txt" },
        ]);
        expect(f.onSubmit.mock.calls[1]![0].idempotency_key).not.toBe(
          f.onSubmit.mock.calls[0]![0].idempotency_key,
        );
      }
    } finally {
      await f.close();
    }
  },
);

it("retains the exact failed request for an explicit retry after a delayed failure", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => reply([])),
  );
  const f = await fixture();
  try {
    const input = f.container.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(() => {
      Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      )!.set!.call(input, "Retain this reviewed request");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    let complete: ((result: boolean) => void) | undefined;
    f.onSubmit.mockImplementationOnce(
      () =>
        new Promise<boolean>((resolve) => {
          complete = resolve;
        }),
    );
    const send = async () =>
      act(async () => {
        f.container
          .querySelector("form")!
          .dispatchEvent(
            new Event("submit", { bubbles: true, cancelable: true }),
          );
      });
    await send();
    await act(() => complete!(false));
    expect(input.value).toBe("Retain this reviewed request");
    await send();
    expect(f.onSubmit.mock.calls[1]![0]).toBe(f.onSubmit.mock.calls[0]![0]);
    expect(input.value).toBe("");
  } finally {
    await f.close();
  }
});

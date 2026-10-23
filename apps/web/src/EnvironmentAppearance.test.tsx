// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { EnvironmentAppearance } from "./Appearance";
import { navigate } from "./navigation";
import type { AdminSettings } from "./control-api";

let root: Root, container: HTMLDivElement;
let metadata: AdminSettings;
let readStatus: number, reads: number;
let writes: {
  body: Record<string, unknown>;
  signal: AbortSignal;
  respond: (body: unknown, status?: number) => void;
}[];
const onSaved = vi.fn(),
  confirm = vi.fn<() => boolean>();
const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  confirm.mockReturnValue(true);
  vi.stubGlobal("confirm", confirm);
  history.replaceState(null, "", "/admin/appearance");
  metadata = {
    revision: 4,
    auth: { local_enabled: true, oidc_label: null },
    classification: {
      enabled: true,
      text: "LOCAL",
      tone: "info",
      position: "top",
      colors: { background: "#123456", text: "#ffffff" },
    },
  };
  readStatus = 200;
  reads = 0;
  writes = [];
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_path: string, init: RequestInit) => {
      if (init.method === "GET") {
        reads++;
        return readStatus === 200
          ? json(metadata)
          : json({ error: "Unavailable" }, readStatus);
      }
      return new Promise<Response>((resolve) =>
        writes.push({
          body: JSON.parse(String(init.body)),
          signal: init.signal as AbortSignal,
          respond: (body, status = 200) => resolve(json(body, status)),
        }),
      );
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function render() {
  await act(() => root.render(<EnvironmentAppearance onSaved={onSaved} />));
}
function button(label: string) {
  const result = [
    ...container.querySelectorAll<HTMLButtonElement>("button"),
  ].find((item) => item.textContent?.trim() === label);
  if (!result) throw new Error(`Missing button ${label}`);
  return result;
}
function color() {
  return container.querySelector<HTMLInputElement>(
    '[aria-label="Classification background color"]',
  )!;
}
async function change(value: string) {
  await act(() => {
    color().value = value;
    color().dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function publish() {
  await act(() => button("Publish banner colors").click());
}
it("publishes one reviewed revision and clears the draft only after acknowledgement", async () => {
  await render();
  await change("#000000");
  await act(() => {
    button("Publish banner colors").click();
    button("Publish banner colors").click();
  });
  expect(writes).toHaveLength(1);
  expect(writes[0]!.body).toEqual({
    revision: 4,
    classification: {
      ...metadata.classification,
      colors: { background: "#000000", text: "#ffffff" },
    },
  });
  expect(button("Reload current banner settings").disabled).toBe(true);
  const acknowledged = {
    revision: 5,
    classification: {
      ...metadata.classification,
      colors: { background: "#000000", text: "#ffffff" },
    },
  };
  await act(() => writes[0]!.respond(acknowledged));
  expect(onSaved).toHaveBeenCalledWith(acknowledged);
  expect(color().value).toBe("#000000");
  confirm.mockReturnValue(false);
  navigate("/settings/account");
  expect(confirm).not.toHaveBeenCalled();
  expect(location.pathname).toBe("/settings/account");
});
it("retains a conflicting proposal and requires an explicit confirmed reload", async () => {
  await render();
  await change("#000000");
  await publish();
  metadata = {
    ...metadata,
    revision: 8,
    classification: {
      ...metadata.classification,
      colors: { background: "#222222", text: "#ffffff" },
    },
  };
  await act(() => writes[0]!.respond({ error: "Conflict" }, 409));
  expect(reads).toBe(1);
  expect(color().value).toBe("#000000");
  expect(button("Publish banner colors").disabled).toBe(true);
  expect(onSaved).not.toHaveBeenCalled();
  confirm.mockReturnValue(false);
  await act(() => button("Reload current banner settings").click());
  expect(reads).toBe(1);
  expect(color().value).toBe("#000000");
  confirm.mockReturnValue(true);
  await act(() => button("Reload current banner settings").click());
  expect(color().value).toBe("#222222");
  await change("#111111");
  await publish();
  expect(writes[1]!.body.revision).toBe(8);
  await act(() =>
    writes[1]!.respond({
      revision: 9,
      classification: {
        ...metadata.classification,
        colors: { background: "#111111", text: "#ffffff" },
      },
    }),
  );
});
it("retains the proposal and publication lock when an explicit reload fails", async () => {
  await render();
  await change("#000000");
  await publish();
  await act(() => writes[0]!.respond({ error: "Unavailable" }, 503));
  readStatus = 503;
  await act(() => button("Reload current banner settings").click());
  expect(color().value).toBe("#000000");
  expect(button("Publish banner colors").disabled).toBe(true);
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "proposed colors are retained",
  );
  expect(onSaved).not.toHaveBeenCalled();
});
it("guards pristine tone-color publication while pending and after an unknown outcome", async () => {
  await render();
  await act(() => button("Use tone colors").click());
  expect(writes[0]!.body).toEqual({
    revision: 4,
    classification: { ...metadata.classification, colors: null },
  });
  confirm.mockReturnValue(false);
  navigate("/settings/account");
  expect(confirm).toHaveBeenCalledOnce();
  expect(location.pathname).toBe("/admin/appearance");
  await act(() => writes[0]!.respond({ error: "Unavailable" }, 503));
  navigate("/settings/account");
  expect(confirm).toHaveBeenCalledTimes(2);
  expect(location.pathname).toBe("/admin/appearance");
});
it("ignores publication completion after the appearance page unmounts", async () => {
  await render();
  await change("#000000");
  await publish();
  await act(() => root.render(<p>Another page</p>));
  expect(writes[0]!.signal.aborted).toBe(true);
  await act(() =>
    writes[0]!.respond({
      revision: 5,
      classification: metadata.classification,
    }),
  );
  expect(onSaved).not.toHaveBeenCalled();
});

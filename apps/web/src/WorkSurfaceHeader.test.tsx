// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import {
  WorkSurfaceHeader,
  WORK_STATUS_PRESENTATIONS,
} from "@colossus/ui/conversation";

let container: HTMLDivElement, root: Root;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});

it("keeps compact header details available with keyboard, outside-pointer dismissal and focus restoration", async () => {
  await act(() =>
    root.render(
      <>
        <WorkSurfaceHeader
          variant="compact"
          title="A conversation"
          statusLabel="Failed"
          status={WORK_STATUS_PRESENTATIONS.failed}
          startedLabel="Oct 21, 5:28 PM"
          modeLabel="Plan mode"
          details={<span>Runtime ready</span>}
        />
        <button>Outside header</button>
      </>,
    ),
  );
  const disclosure = container.querySelector("details")!;
  const summary = container.querySelector("summary")!;
  const outside = container.querySelector("button")!;
  expect(container.querySelector("h2")?.textContent).toBe("A conversation");
  expect(container.querySelector(".surface-breadcrumb")).toBeNull();
  expect(container.querySelector(".surface-run-meta")).toBeNull();
  expect(disclosure.open).toBe(false);
  summary.focus();
  await act(() => summary.click());
  expect(disclosure.open).toBe(true);
  expect(disclosure.textContent).toContain("Work failed");
  expect(disclosure.textContent).toContain("Started Oct 21, 5:28 PM");
  expect(disclosure.textContent).toContain("Plan mode");
  expect(disclosure.textContent).toContain("Runtime ready");
  const escape = new KeyboardEvent("keydown", {
    key: "Escape",
    bubbles: true,
    cancelable: true,
  });
  await act(() => summary.dispatchEvent(escape));
  expect(escape.defaultPrevented).toBe(true);
  expect(disclosure.open).toBe(false);
  expect(document.activeElement).toBe(summary);
  await act(() => summary.click());
  await act(() =>
    outside.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true })),
  );
  expect(disclosure.open).toBe(false);
  await act(() => summary.click());
  await act(() => outside.focus());
  expect(disclosure.open).toBe(false);
  expect(document.activeElement).toBe(outside);
});

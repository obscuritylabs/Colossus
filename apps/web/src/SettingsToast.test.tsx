// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  ToastRegion,
  useToastQueue,
} from "@colossus/ui/components/ToastRegion";

let root: Root;
let container: HTMLDivElement;
function Harness() {
  const { toasts, pushToast, dismissToast, clearToasts } = useToastQueue();
  return (
    <>
      <button onClick={() => pushToast("Credential changes saved.")}>
        Save
      </button>
      <button onClick={() => pushToast("Review the connection.", "error")}>
        Fail
      </button>
      <button onClick={clearToasts}>Change scope</button>
      <ToastRegion
        toasts={toasts}
        onDismiss={dismissToast}
        placement="bottom"
      />
    </>
  );
}
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(() => root.render(<Harness />));
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});
async function click(label: string) {
  const button = [...container.querySelectorAll("button")].find(
    (item) =>
      item.textContent === label || item.getAttribute("aria-label") === label,
  )!;
  await act(() => button.click());
}
it("announces a save without moving focus, then expires the toast", async () => {
  const save = container.querySelector("button")!;
  save.focus();
  await click("Save");
  const region = container.querySelector('[aria-label="Notifications"]')!;
  expect(region.getAttribute("aria-live")).toBe("polite");
  expect(region.getAttribute("data-placement")).toBe("bottom");
  expect(region.textContent).toContain("Credential changes saved.");
  expect(document.activeElement).toBe(save);
  await act(() => vi.advanceTimersByTime(6_000));
  expect(region.textContent).toBe("");
});
it("retains errors for dismissal and clears notifications on scope changes", async () => {
  await click("Fail");
  await act(() => vi.advanceTimersByTime(10_000));
  expect(container.textContent).toContain("Review the connection.");
  await click("Dismiss notification");
  expect(container.textContent).not.toContain("Review the connection.");
  await click("Save");
  await click("Change scope");
  expect(
    container.querySelector('[aria-label="Notifications"]')!.textContent,
  ).toBe("");
});

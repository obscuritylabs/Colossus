// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ClassificationColorsEditor } from "@colossus/ui/appearance";

let root: Root, container: HTMLDivElement;
const initial = { background: "#123456", text: "#ffffff" };
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
it("retains a tracked color draft across publication and callback changes until an explicit reset", async () => {
  const first = vi.fn(),
    next = vi.fn();
  await act(() =>
    root.render(
      <ClassificationColorsEditor
        colors={initial}
        onChange={() => {}}
        onDraftChange={first}
      />,
    ),
  );
  await change("#000000");
  expect(first).toHaveBeenLastCalledWith(true);
  const published = { background: "#222222", text: "#ffffff" };
  await act(() =>
    root.render(
      <ClassificationColorsEditor
        colors={published}
        onChange={() => {}}
        onDraftChange={next}
      />,
    ),
  );
  expect(color().value).toBe("#000000");
  expect(next).not.toHaveBeenCalledWith(false);
  await act(() =>
    root.render(
      <ClassificationColorsEditor
        colors={published}
        onChange={() => {}}
        onDraftChange={next}
        resetKey={1}
      />,
    ),
  );
  expect(color().value).toBe("#222222");
  expect(next).toHaveBeenLastCalledWith(false);
});
it("keeps legacy Desktop prop-controlled behavior without a tracked draft callback", async () => {
  await act(() =>
    root.render(
      <ClassificationColorsEditor colors={initial} onChange={() => {}} />,
    ),
  );
  await change("#000000");
  await act(() =>
    root.render(
      <ClassificationColorsEditor
        colors={{ background: "#222222", text: "#ffffff" }}
        onChange={() => {}}
      />,
    ),
  );
  expect(color().value).toBe("#222222");
});
it("clears tracked changes when a user returns to the reviewed color", async () => {
  const draft = vi.fn();
  await act(() =>
    root.render(
      <ClassificationColorsEditor
        colors={initial}
        onChange={() => {}}
        onDraftChange={draft}
      />,
    ),
  );
  await change("#000000");
  await change(initial.background);
  expect(draft).toHaveBeenLastCalledWith(false);
});

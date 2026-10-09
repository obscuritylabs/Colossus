// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ActivityLineChart } from "@colossus/ui/charts";
import { dashboard } from "./control-api";
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
    function (this: HTMLElement) {
      return this.classList.contains("recharts-responsive-container")
        ? new DOMRect(0, 0, 800, 280)
        : new DOMRect(0, 0, 0, 16);
    },
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});
it("renders genuine line series and retains zero, missing values and UTC dates in its accessible data alternative", async () => {
  const data = [
    { date: "2026-10-01", runs: 0, completed: 0, failed: 0 },
    { date: "2026-10-02", runs: null, completed: null, failed: null },
    { date: "2026-10-03", runs: 8, completed: 7, failed: 1 },
  ];
  await act(() => root.render(<ActivityLineChart data={data} />));
  expect(container.querySelectorAll(".recharts-line")).toHaveLength(3);
  expect(container.querySelector(".recharts-area")).toBeNull();
  const rows = [...container.querySelectorAll("tbody tr")];
  expect(rows).toHaveLength(3);
  expect(
    [...rows[0]!.querySelectorAll("td")].map((cell) => cell.textContent),
  ).toEqual(["0", "0", "0"]);
  expect(
    [...rows[1]!.querySelectorAll("td")].map((cell) => cell.textContent),
  ).toEqual(["Unavailable", "Unavailable", "Unavailable"]);
  expect(rows[2]!.querySelector("time")?.getAttribute("dateTime")).toBe(
    "2026-10-03",
  );
  expect(container.querySelector("caption")?.textContent).toContain("UTC");
  expect(
    container.querySelector(".shared-chart-legend")?.textContent,
  ).toContain("Total runs");
  expect(data[1]!.runs).toBeNull();
});
it("shows an empty state without inventing a chart or daily observations", async () => {
  await act(() => root.render(<ActivityLineChart data={[]} />));
  expect(container.textContent).toContain("No released run activity");
  expect(container.querySelector("svg")).toBeNull();
});
it("constructs bounded dashboard windows without duplicate or unsupported periods", () => {
  expect(dashboard()).toBe("/api/dashboard?days=7");
  expect(dashboard(30)).toBe("/api/dashboard?days=30");
  expect(dashboard(90)).toBe("/api/dashboard?days=90");
  for (const invalid of [0, 91, 7.5, NaN])
    expect(() => dashboard(invalid)).toThrow(RangeError);
});

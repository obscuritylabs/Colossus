// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import {
  ScheduleTiming,
  prepareCalendar,
  type CalendarDraft,
} from "@colossus/ui/automations/ScheduleTiming";
it("preserves native date and time input events in the exact reviewed occurrence", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  (
    globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  function Form() {
    const [draft, setDraft] = useState<CalendarDraft>({
      repeat: "daily",
      date: "2026-10-09",
      time: "09:00",
      timezone: "America/New_York",
      weekdays: [],
    });
    return (
      <>
        <ScheduleTiming value={draft} onChange={setDraft} />
        <output>{prepareCalendar(draft).starts_at}</output>
      </>
    );
  }
  await act(async () => root.render(<Form />));
  for (const [type, value] of [
    ["date", "2030-10-09"],
    ["time", "10:30"],
  ]) {
    await act(async () => {
      const input = container.querySelector<HTMLInputElement>(
        `input[type=${type}]`,
      )!;
      input.value = value!;
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  expect(container.querySelector("output")!.textContent).toBe(
    "2030-10-09T14:30:00.000Z",
  );
  await act(async () => root.unmount());
  container.remove();
});

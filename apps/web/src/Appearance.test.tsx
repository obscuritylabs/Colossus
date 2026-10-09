// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AppearanceSettings } from "./Appearance";
import {
  DEFAULT_THEME_PALETTES,
  parseThemePalettes,
} from "@colossus/ui/lib/palette";

it("validates native color input before persisting browser preferences", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onChange = vi.fn();
  try {
    await act(() =>
      root.render(
        <AppearanceSettings
          appearance={{
            colorTheme: "dark",
            darkPalette: "colossus",
            textSize: "comfortable",
            sendShortcut: "enter",
            palettes: parseThemePalettes(DEFAULT_THEME_PALETTES),
          }}
          onChange={onChange}
        />,
      ),
    );
    const color = container.querySelector<HTMLInputElement>(
      'input[aria-label="Dark background color"]',
    )!;
    await act(() => {
      color.value = "#f0f0f0";
      color.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(onChange).not.toHaveBeenCalled();
    expect(container.textContent).toContain(
      "Choose a darker background color so text and icons stay readable.",
    );
    await act(() => {
      color.value = "#101010";
      color.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange.mock.calls[0]![0].palettes.dark.background).toBe("#101010");
  } finally {
    await act(() => root.unmount());
    container.remove();
  }
});

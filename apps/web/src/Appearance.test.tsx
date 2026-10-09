// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AppearanceSettings, useAppearance } from "./Appearance";
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

it("restores Black without overwriting saved custom colors", async () => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
  });
  const palettes = parseThemePalettes(DEFAULT_THEME_PALETTES);
  palettes.dark.background = "#101010";
  const saved = {
    colorTheme: "dark",
    darkPalette: "black",
    textSize: "large",
    sendShortcut: "modEnter",
    palettes,
  };
  localStorage.setItem("colossus.web.appearance.v1", JSON.stringify(saved));
  function Preferences() {
    const { appearance, setAppearance } = useAppearance();
    return (
      <>
        <AppearanceSettings appearance={appearance} onChange={setAppearance} />
        <button
          data-testid="restore-colossus-palette"
          onClick={() =>
            setAppearance({ ...appearance, darkPalette: "colossus" })
          }
        >
          Restore custom palette
        </button>
      </>
    );
  }
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(() => root.render(<Preferences />));
    expect(document.documentElement.dataset.palette).toBe("black");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(document.documentElement.style.getPropertyValue("--main")).toBe("");
    expect(
      JSON.parse(localStorage.getItem("colossus.web.appearance.v1")!),
    ).toEqual(saved);
    expect(container.textContent).toContain("Black uses preset colors");
    await act(() => {
      container
        .querySelector<HTMLButtonElement>(
          '[data-testid="restore-colossus-palette"]',
        )!
        .click();
    });
    expect(document.documentElement.style.getPropertyValue("--main")).toBe(
      "#101010",
    );
  } finally {
    await act(() => root.unmount());
    container.remove();
    localStorage.removeItem("colossus.web.appearance.v1");
    vi.unstubAllGlobals();
  }
});

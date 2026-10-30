// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { ClassificationColorsEditor } from "@colossus/ui/appearance";
import {
  classificationColorVariables,
  parseClassificationColors,
} from "@colossus/ui/lib/classification-colors";
import { parseThemePalettes } from "@colossus/ui/lib/palette";
import { AppearanceSettings } from "./Appearance";
import { ApiFailure, request } from "./api";

vi.mock("./api", async (original) => ({
  ...(await original<typeof import("./api")>()),
  request: vi.fn(),
}));

afterEach(() => {
  vi.resetAllMocks();
  vi.unstubAllGlobals();
});

const appearance = {
  colorTheme: "dark" as const,
  darkPalette: "colossus" as const,
  textSize: "comfortable" as const,
  sendShortcut: "enter" as const,
  nextPromptMode: "local" as const,
  palettes: parseThemePalettes(null),
};

async function rendered(element: React.ReactNode) {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(() => root.render(element));
  return {
    container,
    dispose: async () => {
      await act(() => root.unmount());
      container.remove();
    },
  };
}

async function inputColor(container: HTMLElement, slot: string, value: string) {
  await act(() => {
    const input = container.querySelector<HTMLInputElement>(
      `input[aria-label="Classification ${slot} color"]`,
    )!;
    input.value = value;
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (candidate) => candidate.textContent?.includes(text),
  )!;
}

it("accepts only literal readable color pairs before exposing CSS variables", () => {
  expect(
    parseClassificationColors({ background: "#008000", text: "#FFFFFF" }),
  ).toEqual({ background: "#008000", text: "#ffffff" });
  for (const colors of [
    null,
    { background: "url(https://example.test)", text: "#ffffff" },
    { background: "#fff", text: "#000000" },
    { background: "#ffffff", text: "#777777" },
    { background: "#000000", text: "var(--text)" },
    { background: "#000000" },
  ]) {
    expect(parseClassificationColors(colors)).toBeNull();
    expect(classificationColorVariables(colors)).toBeUndefined();
  }
  expect(
    parseClassificationColors({ background: "#ffffff", text: "#767676" }),
  ).not.toBeNull();
});

it("previews a readable pair and requires valid contrast before applying or resetting", async () => {
  const onChange = vi.fn();
  const view = await rendered(
    <ClassificationColorsEditor
      colors={{ background: "#008000", text: "#ffffff" }}
      onChange={onChange}
      previewText="INTERNAL"
    />,
  );
  try {
    await inputColor(view.container, "background", "#ffffff");
    expect(button(view.container, "Apply banner colors").disabled).toBe(true);
    expect(view.container.textContent).toContain(
      "at least 4.5:1 contrast before applying",
    );
    expect(onChange).not.toHaveBeenCalled();
    await inputColor(view.container, "text", "#000000");
    const preview = view.container.querySelector<HTMLElement>(
      '[aria-label="Classification color preview"]',
    )!;
    expect(preview.style.getPropertyValue("--classification-background")).toBe(
      "#ffffff",
    );
    expect(preview.textContent).toBe("INTERNAL");
    await act(() => button(view.container, "Apply banner colors").click());
    expect(onChange).toHaveBeenLastCalledWith({
      background: "#ffffff",
      text: "#000000",
    });
    await act(() => button(view.container, "Use tone colors").click());
    expect(onChange).toHaveBeenLastCalledWith(null);
  } finally {
    await view.dispose();
  }
});

it("does not load deployment settings or offer global colors to ordinary users", async () => {
  const view = await rendered(
    <AppearanceSettings
      appearance={appearance}
      onChange={vi.fn()}
      onEnvironmentSettings={vi.fn()}
    />,
  );
  try {
    expect(request).not.toHaveBeenCalled();
    expect(view.container.textContent).not.toContain(
      "Classification banner colors",
    );
  } finally {
    await view.dispose();
  }
});

it("publishes a revision-checked global color change while preserving marking policy", async () => {
  const classification = {
    enabled: true,
    text: "INTERNAL",
    tone: "warning",
    position: "top_and_bottom",
  };
  vi.mocked(request).mockResolvedValueOnce({
    revision: 7,
    classification,
    auth: { local_enabled: true, oidc_label: null },
  });
  vi.mocked(request).mockResolvedValueOnce({
    revision: 8,
    classification: {
      ...classification,
      colors: { background: "#008000", text: "#ffffff" },
    },
  });
  vi.mocked(request).mockResolvedValue({
    revision: 8,
    classification: {
      ...classification,
      colors: { background: "#008000", text: "#ffffff" },
    },
    auth: { local_enabled: true, oidc_label: null },
  });
  const onEnvironmentSettings = vi.fn();
  const view = await rendered(
    <AppearanceSettings
      appearance={appearance}
      onChange={vi.fn()}
      isGlobalAdmin
      onEnvironmentSettings={onEnvironmentSettings}
    />,
  );
  try {
    await inputColor(view.container, "background", "#008000");
    await act(() => button(view.container, "Publish banner colors").click());
    expect(request).toHaveBeenCalledWith(
      "/api/admin/settings",
      {
        revision: 7,
        classification: {
          ...classification,
          colors: { background: "#008000", text: "#ffffff" },
        },
      },
      expect.any(AbortSignal),
      "PATCH",
    );
    expect(onEnvironmentSettings).toHaveBeenCalledWith({
      revision: 8,
      classification: {
        ...classification,
        colors: { background: "#008000", text: "#ffffff" },
      },
    });
    expect(view.container.textContent).toContain(
      "colors published for this Control Plane",
    );
  } finally {
    await view.dispose();
  }
});

it("retains a rejected global proposal until the administrator explicitly reloads the current revision", async () => {
  const classification = {
    enabled: true,
    text: "INTERNAL",
    tone: "warning",
    position: "top_and_bottom",
  };
  vi.mocked(request).mockResolvedValueOnce({
    revision: 7,
    classification,
    auth: { local_enabled: true, oidc_label: null },
  });
  vi.mocked(request).mockRejectedValueOnce(
    new ApiFailure(
      409,
      "The environment changed. Review its current settings.",
    ),
  );
  vi.mocked(request).mockResolvedValue({
    revision: 8,
    classification: {
      ...classification,
      colors: { background: "#000000", text: "#ffffff" },
    },
    auth: { local_enabled: true, oidc_label: null },
  });
  const onEnvironmentSettings = vi.fn();
  const view = await rendered(
    <AppearanceSettings
      appearance={appearance}
      onChange={vi.fn()}
      isGlobalAdmin
      onEnvironmentSettings={onEnvironmentSettings}
    />,
  );
  try {
    await inputColor(view.container, "background", "#008000");
    await act(() => button(view.container, "Publish banner colors").click());
    expect(onEnvironmentSettings).not.toHaveBeenCalled();
    expect(
      view.container.querySelector('[role="alert"]')?.textContent,
    ).toContain("Another administrator changed these settings");
    expect(view.container.textContent).not.toContain(
      "colors published for this Control Plane",
    );
    expect(request).toHaveBeenCalledTimes(2);
    expect(button(view.container, "Publish banner colors").disabled).toBe(true);
    expect(
      view.container.querySelector<HTMLInputElement>(
        'input[aria-label="Classification background color"]',
      )!.value,
    ).toBe("#008000");
    const confirm = vi.fn(() => true);
    vi.stubGlobal("confirm", confirm);
    await act(() =>
      button(view.container, "Reload current banner settings").click(),
    );
    expect(confirm).toHaveBeenCalledWith(
      "Discard your proposed banner colors and reload current settings?",
    );
    expect(request).toHaveBeenCalledTimes(3);
    expect(
      view.container.querySelector<HTMLInputElement>(
        'input[aria-label="Classification background color"]',
      )!.value,
    ).toBe("#000000");
    expect(onEnvironmentSettings).not.toHaveBeenCalled();
  } finally {
    await view.dispose();
  }
});

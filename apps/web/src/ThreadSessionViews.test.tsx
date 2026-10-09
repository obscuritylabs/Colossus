// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ThreadSessionViews } from "./ThreadSessionViews";
import type { Update } from "./api";

it("distinguishes incomplete artifact coverage and opens only released metadata", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const props = {
    view: "resources" as const,
    tasks: [],
    updates: [] as Update[],
    onChangeView: vi.fn(),
    onRevisePlan: vi.fn(),
    canContinuePlan: false,
  };
  try {
    await act(() => root.render(<ThreadSessionViews {...props} />));
    expect(container.textContent).toContain(
      "No artifact metadata in the loaded messages.",
    );
    expect(container.textContent).not.toContain(
      "No artifacts have been released.",
    );
    expect(container.textContent).toContain(
      "Local files and earlier history may be available in Desktop.",
    );
    const event: Update = {
      run_id: "run",
      sequence: 1,
      created_at: "2026-10-08T12:00:00Z",
      update: {
        message: {
          content: [
            {
              artifact: {
                artifact_id: "artifact-one",
                file_name: "report.json",
                media_type: "application/json",
                byte_length: 44,
                state: "released",
              },
            },
          ],
        },
      },
    };
    await act(() =>
      root.render(<ThreadSessionViews {...props} updates={[event]} />),
    );
    const button = [...container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("report.json"),
    );
    expect(button).toBeDefined();
    await act(() => button!.click());
    const dialog = document.querySelector("dialog");
    expect(dialog?.textContent).toContain("application/json");
    expect(dialog?.textContent).toContain("44 bytes");
    expect(dialog?.textContent).toContain(
      "Open local file contents in Desktop.",
    );
  } finally {
    await act(() => root.unmount());
    container.remove();
  }
});

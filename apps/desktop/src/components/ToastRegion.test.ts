import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { ToastRegion } from "./ToastRegion";

describe("ToastRegion", () => {
  it("renders bounded accessible notifications with dismissal controls", () => {
    const markup = renderToStaticMarkup(
      createElement(ToastRegion, {
        toasts: [
          {
            id: 1,
            message: "confluence is healthy; 3 tools discovered.",
            tone: "success",
          },
          {
            id: 2,
            message: "Configuration still needs attention.",
            tone: "error",
          },
        ],
        onDismiss: vi.fn(),
      }),
    );

    expect(markup).toContain('role="region"');
    expect(markup).toContain('aria-label="Notifications"');
    expect(markup).toContain('aria-live="polite"');
    expect(markup).toContain("confluence is healthy; 3 tools discovered.");
    expect(markup).toContain("is-success");
    expect(markup).toContain("is-error");
    expect(markup.match(/aria-label="Dismiss notification"/g)).toHaveLength(2);
  });
});

// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ConversationEntry,
  sumConversationUsage,
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
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
const copyButton = () =>
  container.querySelector<HTMLButtonElement>(
    ".shared-message-footer .shared-message-copy",
  )!;

describe("shared message utilities", () => {
  it("uses the host-provided assistant asset without changing marker geometry", async () => {
    await act(() =>
      root.render(
        <ConversationEntry
          role="assistant"
          content="Released reply"
          assistantMarkSrc="/allowed/colossus-mark.svg"
        />,
      ),
    );
    const mark = container.querySelector<HTMLImageElement>(
      ".shared-assistant-marker img",
    )!;
    expect(mark.getAttribute("src")).toBe("/allowed/colossus-mark.svg");
    expect(mark.width).toBe(17);
    expect(mark.height).toBe(17);
    expect(mark.alt).toBe("");
  });

  it("copies the exact released Markdown only after activation and reports failure and retry", async () => {
    const text = "**Released response**\n\n```py\nprint('hello')\n```";
    const copy = vi
      .fn()
      .mockRejectedValueOnce(new Error("denied"))
      .mockResolvedValueOnce(undefined);
    await act(() =>
      root.render(
        <ConversationEntry role="assistant" content={text} onCopy={copy} />,
      ),
    );
    expect(copy).not.toHaveBeenCalled();
    await act(async () => copyButton().click());
    expect(copy).toHaveBeenCalledWith(text);
    expect(copyButton().getAttribute("aria-label")).toBe(
      "Could not copy Colossus response; retry",
    );
    await act(async () => copyButton().click());
    expect(copyButton().getAttribute("aria-label")).toBe(
      "Copied Colossus response",
    );
    expect(
      copyButton().querySelector('[aria-live="polite"]')?.textContent,
    ).toContain("copied to clipboard");
  });

  it("ignores clipboard completion for a replaced message", async () => {
    let finish!: () => void;
    const copy = () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      });
    await act(() =>
      root.render(
        <ConversationEntry role="user" content="Earlier" onCopy={copy} />,
      ),
    );
    await act(async () => copyButton().click());
    await act(() =>
      root.render(
        <ConversationEntry role="user" content="Current" onCopy={copy} />,
      ),
    );
    await act(async () => finish());
    expect(copyButton().getAttribute("aria-label")).toBe("Copy message");
  });

  it("preserves keyboard focus and the selected conversation when an embedded clipboard needs fallback", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("unsupported"));
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const fallback = vi.fn().mockReturnValue(true);
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: fallback,
    });
    await act(() =>
      root.render(<ConversationEntry role="user" content="Released text" />),
    );
    const text = container.querySelector(".shared-preserve-lines")!.firstChild!;
    const selection = document.getSelection()!;
    const range = document.createRange();
    range.selectNodeContents(text);
    selection.removeAllRanges();
    selection.addRange(range);
    copyButton().focus();
    await act(async () => copyButton().click());
    expect(writeText).toHaveBeenCalledWith("Released text");
    expect(fallback).toHaveBeenCalledWith("copy");
    expect(document.activeElement).toBe(copyButton());
    expect(selection.toString()).toBe("Released text");
    expect(document.querySelector(".shared-copy-buffer")).toBeNull();
  });

  it("keeps exact counts and optional usage details without inventing missing accounting", async () => {
    const usage = sumConversationUsage([
      {
        inputTokens: "9007199254740993",
        outputTokens: 3,
        totalTokens: "9007199254740996",
        cachedInputTokens: 2,
      },
      { inputTokens: 10, outputTokens: 5, totalTokens: 15 },
    ]);
    expect(usage).toEqual({
      inputTokens: "9007199254741003",
      outputTokens: "8",
      totalTokens: "9007199254741011",
    });
    await act(() =>
      root.render(
        <ConversationEntry role="assistant" content="Released" usage={usage} />,
      ),
    );
    expect(
      container.querySelector(".shared-message-usage")?.textContent,
    ).toContain("9,007,199,254,741,011 tokens");
    expect(
      container.querySelector(".shared-message-usage-details")?.textContent,
    ).toContain("Reported run usage");
    expect(
      container.querySelector(".shared-message-usage-details")?.textContent,
    ).not.toContain("Cached input tokens");
    expect(sumConversationUsage([])).toBeNull();
    expect(
      sumConversationUsage([
        { inputTokens: -1, outputTokens: 2, totalTokens: 1 },
      ]),
    ).toBeNull();
    await act(() =>
      root.render(
        <ConversationEntry role="assistant" content="Without telemetry" />,
      ),
    );
    expect(container.querySelector(".shared-message-usage")).toBeNull();
  });
});

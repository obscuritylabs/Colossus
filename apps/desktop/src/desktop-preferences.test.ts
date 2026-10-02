import { describe, expect, it } from "vitest";
import {
  DEFAULT_DESKTOP_PREFERENCES,
  parseDesktopPreferences,
  validBrowserNewTabUrl,
} from "./desktop-preferences";

describe("Desktop tool preferences", () => {
  it("rejects unsafe new-tab addresses and stale stored values", () => {
    for (const address of [
      "javascript:alert(1)",
      "file:///etc/passwd",
      "https://user:secret@example.com/",
      "https://example.com/\nother",
    ]) {
      expect(validBrowserNewTabUrl(address)).toBeNull();
      expect(
        parseDesktopPreferences(JSON.stringify({ browserNewTabUrl: address }))
          .browserNewTabUrl,
      ).toBe("");
    }
    expect(validBrowserNewTabUrl("http://localhost:3000/")).toBe(
      "http://localhost:3000/",
    );
  });

  it("keeps safe defaults when storage is missing or damaged", () => {
    expect(parseDesktopPreferences(null)).toEqual(DEFAULT_DESKTOP_PREFERENCES);
    expect(parseDesktopPreferences("{")).toEqual(DEFAULT_DESKTOP_PREFERENCES);
    expect(
      parseDesktopPreferences(
        JSON.stringify({
          gitAutoRefresh: false,
          gitDefaultView: "history",
          browserNewTabUrl: "https://example.com",
          terminalDefaultSession: "shell",
        }),
      ),
    ).toEqual({
      gitAutoRefresh: false,
      gitDefaultView: "history",
      browserNewTabUrl: "https://example.com/",
      terminalDefaultSession: "shell",
    });
  });
});

import { describe, expect, it } from "vitest";

import { isNearConversationLatest } from "./conversation-follow";

describe("conversation follow", () => {
  it("keeps following while the reader remains near the latest content", () => {
    expect(
      isNearConversationLatest({
        scrollTop: 904,
        scrollHeight: 1500,
        clientHeight: 500,
      }),
    ).toBe(true);
  });

  it("pauses follow after the reader moves beyond the bottom threshold", () => {
    expect(
      isNearConversationLatest({
        scrollTop: 800,
        scrollHeight: 1500,
        clientHeight: 500,
      }),
    ).toBe(false);
  });
});

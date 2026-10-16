import { describe, expect, it } from "vitest";
import { normalizeOrigin, runRequest } from "./client";

describe("human server selection", () => {
  it("accepts only a credential-free HTTPS origin in release", () => {
    expect(normalizeOrigin(" https://colossus.example:8443/ ")).toBe(
      "https://colossus.example:8443",
    );
    for (const address of [
      "http://example.com",
      "https://user:secret@example.com",
      "https://example.com/api",
      "https://example.com?token=secret",
      "https://example.com/#secret",
      "javascript:alert(1)",
    ])
      expect(() => normalizeOrigin(address)).toThrow();
  });
  it("limits the development HTTP exception to actual loopback", () => {
    expect(normalizeOrigin("http://localhost:8090", true)).toBe(
      "http://localhost:8090",
    );
    expect(() =>
      normalizeOrigin("http://localhost.example:8090", true),
    ).toThrow();
    expect(() => normalizeOrigin("http://10.0.2.2:8090", true)).toThrow();
  });
  it("retains the reviewed allocation identity and a bounded execution request", () => {
    const request = runRequest(
      "Review cancellation",
      "primary",
      "plan",
      "original-attempt",
    );
    expect(request.idempotency_key).toBe("original-attempt");
    expect(request.session_id).toBeNull();
    expect(request.max_turns).toBe(24);
    expect(request.mode).toBe("plan");
  });
});

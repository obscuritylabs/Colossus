import { describe, expect, it } from "vitest";

import {
  LOOPBACK_PROVIDER_TIMEOUT_MS,
  REMOTE_PROVIDER_TIMEOUT_MS,
  automaticProviderTimeoutMs,
} from "./providerTimeout";

describe("automaticProviderTimeoutMs", () => {
  it("uses the longer default only for loopback hosts", () => {
    for (const url of [
      "http://localhost:11434/v1",
      "http://127.1:11434/v1",
      "http://[::1]:11434/v1",
      "https://127.0.0.1/v1",
    ]) {
      expect(automaticProviderTimeoutMs(url)).toBe(
        LOOPBACK_PROVIDER_TIMEOUT_MS,
      );
    }
    expect(automaticProviderTimeoutMs("https://models.example.test/v1")).toBe(
      REMOTE_PROVIDER_TIMEOUT_MS,
    );
    expect(automaticProviderTimeoutMs("https://192.168.1.10/v1")).toBe(
      REMOTE_PROVIDER_TIMEOUT_MS,
    );
  });
});

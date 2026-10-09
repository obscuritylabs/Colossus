import { describe, expect, it } from "vitest";
import { cloudConnectionError } from "./cloud-connection-errors";

const nativeError = {
  code: "invalid_argument",
  message: "The request is invalid.",
  retryable: false,
  outcomeUnknown: false,
  violations: [
    {
      field: "cloud_connection",
      description: "pending enrollment belongs to another runtime or cloud",
    },
  ],
};

describe("Control Plane connection errors", () => {
  it("shows the native enrollment rejection instead of the generic message", () => {
    expect(cloudConnectionError(nativeError, "Unavailable")).toBe(
      "pending enrollment belongs to another runtime or cloud",
    );
  });

  it("retains confirmed errors without field violations", () => {
    expect(
      cloudConnectionError(
        {
          ...nativeError,
          message: "The enrollment vault is unavailable.",
          violations: [],
        },
        "Unavailable",
      ),
    ).toBe("The enrollment vault is unavailable.");
  });

  it("ignores malformed or empty field violations", () => {
    expect(
      cloudConnectionError(
        {
          ...nativeError,
          violations: [
            null,
            { description: 12 },
            { field: "url", description: " " },
          ],
        },
        "Unavailable",
      ),
    ).toBe("The request is invalid.");
  });

  it("uses the operation fallback for an unstructured bridge error", () => {
    expect(cloudConnectionError("raw bridge error", "Unavailable")).toBe(
      "Unavailable",
    );
  });
});

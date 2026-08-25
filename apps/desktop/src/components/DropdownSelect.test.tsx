import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import {
  DropdownSelect,
  dropdownOptions,
  nextDropdownOptionIndex,
} from "./DropdownSelect";

describe("DropdownSelect", () => {
  it("renders an app-owned accessible trigger without a native select", () => {
    const markup = renderToStaticMarkup(
      createElement(
        DropdownSelect,
        {
          value: "http_protobuf",
          "aria-label": "Protocol",
          onChange: vi.fn(),
        },
        createElement("option", { value: "grpc" }, "OTLP gRPC"),
        createElement(
          "option",
          { value: "http_protobuf" },
          "OTLP HTTP/protobuf",
        ),
      ),
    );

    expect(markup).toContain('role="combobox"');
    expect(markup).toContain('aria-label="Protocol"');
    expect(markup).toContain('aria-haspopup="listbox"');
    expect(markup).toContain("OTLP HTTP/protobuf");
    expect(markup).not.toContain("<select");
    expect(markup).not.toContain("<option");
  });

  it("normalizes option children and skips disabled choices while moving", () => {
    const options = dropdownOptions([
      createElement("option", { value: "one", key: "one" }, "One"),
      createElement(
        "option",
        { value: "two", disabled: true, key: "two" },
        "Two",
      ),
      createElement("option", { value: "three", key: "three" }, "Three"),
    ]);

    expect(options).toEqual([
      { value: "one", label: "One", disabled: false },
      { value: "two", label: "Two", disabled: true },
      { value: "three", label: "Three", disabled: false },
    ]);
    expect(nextDropdownOptionIndex(options, 0, 1)).toBe(2);
    expect(nextDropdownOptionIndex(options, 0, -1)).toBe(2);
  });

  it("preserves required form validation without rendering a native select", () => {
    const markup = renderToStaticMarkup(
      createElement(
        DropdownSelect,
        {
          value: "",
          required: true,
          "aria-label": "Provider",
          onChange: vi.fn(),
        },
        createElement("option", { value: "" }, "Select a provider"),
        createElement("option", { value: "primary" }, "Primary provider"),
      ),
    );

    expect(markup).toContain('aria-required="true"');
    expect(markup).toContain('aria-invalid="true"');
    expect(markup).toContain('class="app-select-validity-proxy"');
    expect(markup).toContain("required");
    expect(markup).not.toContain("<select");
  });

  it("shows an unavailable controlled value instead of a different option", () => {
    const markup = renderToStaticMarkup(
      createElement(
        DropdownSelect,
        {
          value: "removed-provider",
          "aria-label": "Provider",
          onChange: vi.fn(),
        },
        createElement("option", { value: "primary" }, "Primary provider"),
      ),
    );

    expect(markup).toContain("Unavailable: removed-provider");
    expect(markup).not.toContain(">Primary provider<");
  });
});

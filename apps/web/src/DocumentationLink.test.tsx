// @vitest-environment happy-dom
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { documentationUrl, DocumentationLink } from "./DocumentationLink";

it("supports same-origin mounts and HTTP(S) documentation without unsafe URL schemes", () => {
  expect(documentationUrl(undefined)).toBe("/docs/");
  expect(documentationUrl("/reference/?q=runtime#agents")).toBe(
    "/reference/?q=runtime#agents",
  );
  expect(documentationUrl(" https://docs.example.test/reference/ ")).toBe(
    "https://docs.example.test/reference/",
  );
  expect(documentationUrl("http://localhost:8093/docs/")).toBe(
    "http://localhost:8093/docs/",
  );
  for (const value of [
    "",
    "javascript:alert(1)",
    "data:text/html,<script>alert(1)</script>",
    "//docs.example.test/",
    "/\\docs.example.test/",
    "https://user:password@docs.example.test/",
    "https://docs.example.test/\nreference/",
    "/docs/\u0000reference/",
    "https:docs.example.test",
    "./docs/",
    "/" + "a".repeat(2048),
  ])
    expect(documentationUrl(value)).toBe("/docs/");
});

it("renders an ordinary labelled documentation anchor that preserves the current page", () => {
  const container = document.createElement("div");
  container.innerHTML = renderToStaticMarkup(
    <DocumentationLink configuredUrl="https://docs.example.test/" />,
  );
  const link = container.querySelector("a");
  expect(link?.getAttribute("href")).toBe("https://docs.example.test/");
  expect(link?.getAttribute("aria-label")).toBe(
    "Documentation (opens in a new tab)",
  );
  expect(link?.getAttribute("target")).toBe("_blank");
  expect(link?.getAttribute("rel")).toBe("noopener noreferrer");
  expect(link?.classList.contains("ui-button--icon")).toBe(true);
  expect(link?.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
});

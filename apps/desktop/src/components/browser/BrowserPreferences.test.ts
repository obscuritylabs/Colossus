import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  BrowserPreferences,
  validBrowserUserAgent,
} from "./BrowserPreferences";
import {
  BrowserConnectionDetails,
  nativeNavigationError,
} from "./BrowserConnectionDetails";

describe("human browser profile", () => {
  it("rejects header injection and ambiguous or unbounded native user agents", () => {
    expect(validBrowserUserAgent("")).toBe(true);
    expect(validBrowserUserAgent("Mozilla/5.0 (Colossus fixture)")).toBe(true);
    for (const value of [
      "test\r\nAuthorization: hidden",
      "\tfoo",
      " foo",
      "foo ",
      "é",
      "a".repeat(513),
    ])
      expect(validBrowserUserAgent(value)).toBe(false);
  });
  it("explains the explicit engine choice, cookie isolation and native clear controls", () => {
    const html = renderToStaticMarkup(createElement(BrowserPreferences));
    expect(html).toContain("Remember sign-ins");
    expect(html).toContain("Windows WebView2");
    expect(html).toContain("Agent sessions never receive these cookies");
    expect(html).toContain("Clear browsing data");
    expect(html).toContain("Reset browser profile");
  });
  it("shows bounded authentication evidence without suggesting a TLS bypass", () => {
    const html = renderToStaticMarkup(
      createElement(BrowserConnectionDetails, {
        onClose() {},
        details: {
          origin: "https://fixture.test",
          httpStatus: 403,
          redirects: 2,
          nativeError: null,
          clientCertificateRequested: true,
          tlsFailed: false,
          persistent: true,
          customUserAgent: true,
          systemEngine: true,
        },
      }),
    );
    expect(html).toContain("HTTP 403");
    expect(html).toContain("Windows certificate chooser");
    expect(html).toContain("Custom native value applied");
    expect(html).toContain("native trust policy");
    expect(html).toContain("cannot guarantee access");
  });
});

it("describes native authentication and network failures without raw engine messages", () => {
  expect(nativeNavigationError(17)).toBe(
    "Sign-in credentials required (code 17)",
  );
  expect(nativeNavigationError(18)).toBe(
    "Proxy authentication required (code 18)",
  );
  expect(nativeNavigationError(13)).toBe(
    "Hostname could not be resolved (code 13)",
  );
  expect(nativeNavigationError(999)).toBe("Engine error (code 999)");
  expect(nativeNavigationError(null)).toBe("None reported");
});

it("does not label the existing Mac system view as WebView2 or claim unsupported redirect metadata", () => {
  const html = renderToStaticMarkup(
    createElement(BrowserConnectionDetails, {
      onClose() {},
      engineKind: "webkit",
      details: {
        origin: "https://fixture.test",
        httpStatus: null,
        redirects: 0,
        nativeError: null,
        clientCertificateRequested: false,
        tlsFailed: false,
        persistent: false,
        customUserAgent: false,
        systemEngine: true,
      },
    }),
  );
  expect(html).toContain("WebKit");
  expect(html).not.toContain("Windows WebView2");
  expect(html).toContain("Not reported by this engine");
});

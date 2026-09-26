import { createContext, useContext } from "react";
import type { ReactNode } from "react";

export const BrowserLinkContext = createContext<((url: string) => void) | null>(
  null,
);

// This only controls presentation. Native navigation validates the destination
// again and applies app-origin and per-tab loopback rules.
export function webLink(value: string | undefined): string | null {
  if (!value || value.length > 8192 || /[\u0000-\u0020\\]/.test(value))
    return null;
  try {
    const url = new URL(value);
    return ["https:", "http:"].includes(url.protocol) &&
      !url.username &&
      !url.password
      ? url.href
      : null;
  } catch {
    return null;
  }
}

export function BrowserLink({
  href,
  children,
}: {
  href?: string | undefined;
  children?: ReactNode;
}) {
  const open = useContext(BrowserLinkContext);
  const url = webLink(href);
  return open && url ? (
    <button
      className="browser-content-link"
      type="button"
      title={`Open in browser: ${url}`}
      onClick={() => open(url)}
    >
      {children}
      <span className="sr-only"> (open in browser)</span>
    </button>
  ) : (
    <span className="markdown-link-inert">{children}</span>
  );
}

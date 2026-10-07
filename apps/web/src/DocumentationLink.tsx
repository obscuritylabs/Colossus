import { Button } from "@colossus/ui/components/ui/button";
import { IconBook2 } from "@tabler/icons-react";

const defaultDocumentationUrl = "/docs/";

export function documentationUrl(configured: unknown): string {
  if (typeof configured !== "string") return defaultDocumentationUrl;
  const candidate = configured.trim();
  if (
    !candidate ||
    candidate.length > 2048 ||
    /[\s\\\u0000-\u001f\u007f]/u.test(candidate)
  )
    return defaultDocumentationUrl;
  if (candidate.startsWith("/") && !candidate.startsWith("//"))
    return candidate;
  if (!/^https?:\/\//i.test(candidate)) return defaultDocumentationUrl;
  try {
    const url = new URL(candidate);
    if (
      (url.protocol === "https:" || url.protocol === "http:") &&
      !url.username &&
      !url.password
    )
      return url.href;
  } catch {
    // An invalid deployment override uses the same-origin documentation mount.
  }
  return defaultDocumentationUrl;
}

export function DocumentationLink({
  configuredUrl = import.meta.env.VITE_COLOSSUS_DOCUMENTATION_URL,
}: {
  configuredUrl?: string | undefined;
}) {
  return (
    <Button asChild variant="ghost" size="icon">
      <a
        href={documentationUrl(configuredUrl)}
        target="_blank"
        rel="noopener noreferrer"
        aria-label="Documentation (opens in a new tab)"
        title="Documentation (opens in a new tab)"
      >
        <IconBook2 size={19} aria-hidden="true" />
      </a>
    </Button>
  );
}

import type { ReactNode } from "react";
import { safeWebLink } from "@colossus/ui/conversation";
/** Browser navigation remains a web-host decision; generated links receive no application authority. */
export function WebLink({
  href,
  children,
}: {
  href?: string | undefined;
  children?: ReactNode;
}) {
  const safe = safeWebLink(href);
  return safe ? (
    <a href={safe} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ) : (
    <span>{children}</span>
  );
}

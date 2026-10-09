import { createContext, useContext, type ReactNode } from "react";
import {
  MarkdownContent as SharedMarkdown,
  type MarkdownLink,
} from "../components/MarkdownContent";
const Links = createContext<MarkdownLink | undefined>(undefined);
/** Host-owned navigation for released markdown, inert when no adapter is supplied. */
export function SessionPresentationProvider({
  linkComponent,
  children,
}: {
  linkComponent: MarkdownLink;
  children: ReactNode;
}) {
  return <Links.Provider value={linkComponent}>{children}</Links.Provider>;
}
export function MarkdownContent({
  content,
  className,
}: {
  content: string;
  className?: string;
}) {
  return (
    <SharedMarkdown
      content={content}
      className={className ?? ""}
      linkComponent={useContext(Links)}
    />
  );
}

import { memo } from "react";
import {
  MarkdownContent as SharedMarkdownContent,
  markdownContentPropsAreEqual,
} from "@colossus/ui/conversation";
import { BrowserLink } from "./browser/BrowserLink";
export {
  MAX_MARKDOWN_CHARACTERS,
  MAX_MARKDOWN_AST_NODES,
  markdownContentPropsAreEqual,
} from "@colossus/ui/conversation";
interface MarkdownContentProps {
  content: string;
  className?: string;
}
export const MarkdownContent = memo(function MarkdownContent({
  content,
  className,
}: MarkdownContentProps) {
  return (
    <SharedMarkdownContent
      content={content}
      className={className ?? ""}
      linkComponent={BrowserLink}
    />
  );
}, markdownContentPropsAreEqual);

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
  isStreaming?: boolean;
}
export const MarkdownContent = memo(function MarkdownContent({
  content,
  className,
  isStreaming,
}: MarkdownContentProps) {
  return (
    <SharedMarkdownContent
      content={content}
      isStreaming={isStreaming}
      className={className ?? ""}
      linkComponent={BrowserLink}
    />
  );
}, markdownContentPropsAreEqual);

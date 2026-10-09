import {
  memo,
  type ReactNode,
  type RefObject,
  type HTMLAttributes,
} from "react";
import { IconMessageCircle } from "@tabler/icons-react";
import colossusMark from "../../assets/colossus-mark.svg";
import { MarkdownContent, type MarkdownLink } from "./MarkdownContent.js";

export interface ConversationEntryProps {
  role: "user" | "assistant" | "tool" | "system";
  author?: string;
  createdAt?: string;
  content?: string;
  markdown?: boolean;
  streaming?: boolean;
  status?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
  linkComponent?: MarkdownLink | undefined;
  className?: string;
}
export const ConversationEntry = /* @__PURE__ */ memo(
  function ConversationEntry({
    role,
    author,
    createdAt,
    content,
    markdown = role === "assistant",
    streaming = false,
    status,
    actions,
    children,
    linkComponent,
    className = "",
  }: ConversationEntryProps) {
    const title =
      author ??
      (role === "user"
        ? "You"
        : role === "assistant"
          ? "Colossus"
          : role === "tool"
            ? "Tool"
            : "System");
    const timestamp = createdAt ? new Date(createdAt) : null;
    return (
      <article
        className={`shared-conversation-entry shared-message-${role} ${className}`}
        data-role={role}
        aria-busy={streaming}
      >
        <div
          className={`shared-feed-marker ${role === "assistant" ? "shared-assistant-marker" : ""}`}
          aria-hidden="true"
        >
          {role === "assistant" ? (
            <img src={colossusMark} alt="" width={17} height={17} />
          ) : (
            <IconMessageCircle size={17} stroke={1.7} />
          )}
        </div>
        <div className="shared-feed-content">
          <header className="shared-feed-heading">
            <strong>{title}</strong>
            <div>
              {status}
              {timestamp && Number.isFinite(timestamp.getTime()) ? (
                <time dateTime={createdAt}>
                  {timestamp.toLocaleTimeString([], {
                    hour: "numeric",
                    minute: "2-digit",
                  })}
                </time>
              ) : null}
            </div>
          </header>
          <div className="shared-message-body" data-aside-selectable="true">
            {content !== undefined ? (
              markdown ? (
                <MarkdownContent
                  content={content}
                  linkComponent={linkComponent}
                />
              ) : (
                <div className="shared-preserve-lines">{content}</div>
              )
            ) : null}
            {children}
          </div>
          {actions ? (
            <div className="shared-message-actions">{actions}</div>
          ) : null}
        </div>
      </article>
    );
  },
);

/** Hosts own cursors, scroll decisions, capabilities, and the released-data projection. */
export function ConversationTimeline({
  children,
  className = "",
  ref,
  ...props
}: HTMLAttributes<HTMLDivElement> & {
  ref?: RefObject<HTMLDivElement | null>;
}) {
  return (
    <div
      {...props}
      ref={ref}
      className={`shared-conversation-timeline ${className}`}
    >
      {children}
    </div>
  );
}

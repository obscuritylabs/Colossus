import {
  Children,
  Fragment,
  isValidElement,
  lazy,
  Suspense,
  useEffect,
  useState,
} from "react";
import type { ReactNode } from "react";
import { IconCode, IconSchema, IconTextWrap } from "@tabler/icons-react";
import { Button } from "./ui/button.js";
import { MessageCopyButton } from "./MessageCopyButton.js";
import type { CodeToken } from "../markdown/highlight.js";

function UnavailableDiagram() {
  return (
    <p className="markdown-mermaid-notice" role="note">
      Unable to load the diagram preview. View the source instead.
    </p>
  );
}
const MermaidPreview = lazy(() =>
  import("../markdown/MermaidPreview.js").catch(() => ({
    default: UnavailableDiagram,
  })),
);
let nextCodeBlock = 1;

function codeText(children: ReactNode): string {
  return Children.toArray(children)
    .map((child) => {
      if (typeof child === "string" || typeof child === "number")
        return String(child);
      if (isValidElement<{ children?: ReactNode }>(child))
        return codeText(child.props.children);
      return "";
    })
    .join("");
}

export function MarkdownCodeBlock({
  children,
  isStreaming = false,
}: {
  children?: ReactNode;
  isStreaming?: boolean | undefined;
}) {
  const [ordinal] = useState(() => nextCodeBlock++);
  const child = Children.toArray(children).find(isValidElement);
  const className = isValidElement<{ className?: string }>(child)
    ? child.props.className
    : "";
  const language =
    /(?:^|\s)language-([^\s]+)/.exec(className ?? "")?.[1]?.slice(0, 40) ??
    "text";
  const source = codeText(children).replace(/\n$/, "");
  const [highlighted, setHighlighted] = useState<{
    source: string;
    language: string;
    lines: CodeToken[][];
  } | null>(null);
  const [wrap, setWrap] = useState(false);
  const [view, setView] = useState<"preview" | "code">("preview");
  const isMermaid = language.toLowerCase() === "mermaid";
  useEffect(() => {
    if (isMermaid || language === "text" || language === "plaintext") return;
    let disposed = false;
    // Coalesce small streaming deltas. Stale work can never replace newer text.
    const timer = setTimeout(
      () => {
        void import("../markdown/highlight.js")
          .then((m) => m.highlightCode(source, language))
          .then((lines) => {
            if (!disposed && lines) setHighlighted({ source, language, lines });
          })
          .catch(() => {
            /* Plain source remains available if a grammar fails. */
          });
      },
      isStreaming ? 120 : 0,
    );
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [source, language, isMermaid, isStreaming]);
  const lines =
    highlighted?.source === source && highlighted.language === language
      ? highlighted.lines
      : null;
  return (
    <section
      className="markdown-code-block"
      aria-label={`${language} code block`}
    >
      <div className="markdown-code-header">
        <span className="markdown-code-language">
          <IconCode size={14} aria-hidden="true" />
          {language}
        </span>
        <div className="markdown-code-actions">
          {isMermaid && (
            <Button
              variant="ghost"
              size="sm"
              aria-pressed={view === "preview"}
              onClick={() => setView(view === "preview" ? "code" : "preview")}
            >
              {view === "preview" ? (
                <IconCode size={14} aria-hidden="true" />
              ) : (
                <IconSchema size={14} aria-hidden="true" />
              )}
              {view === "preview" ? "Source" : "Preview"}
            </Button>
          )}
          {(!isMermaid || view === "code") && (
            <Button
              variant="ghost"
              size="sm"
              aria-label="Wrap code"
              aria-pressed={wrap}
              onClick={() => setWrap(!wrap)}
            >
              <IconTextWrap size={14} aria-hidden="true" />
            </Button>
          )}
          <MessageCopyButton text={source} label="code" />
        </div>
      </div>
      {isMermaid && view === "preview" ? (
        <Suspense
          fallback={<p className="markdown-mermaid-notice">Loading diagram…</p>}
        >
          <MermaidPreview source={source} isStreaming={isStreaming} />
        </Suspense>
      ) : (
        <pre
          role="region"
          aria-label={`Scrollable Markdown code block ${ordinal}`}
          tabIndex={0}
          data-wrap={wrap}
          dir="ltr"
        >
          <code className={`language-${language}`}>
            {lines
              ? lines.map((line, index) => (
                  <Fragment key={index}>
                    {index > 0 ? "\n" : null}
                    {line.map((token, tokenIndex) => (
                      <span className={token.className} key={tokenIndex}>
                        {token.content}
                      </span>
                    ))}
                  </Fragment>
                ))
              : source}
          </code>
        </pre>
      )}
    </section>
  );
}

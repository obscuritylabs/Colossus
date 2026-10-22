import {
  Children,
  isValidElement,
  memo,
  useId,
  useMemo,
  useState,
} from "react";
import type { ComponentPropsWithoutRef, ComponentType, ReactNode } from "react";
import rehypeSanitize from "rehype-sanitize";
import { Streamdown } from "streamdown";
import { MarkdownCodeBlock } from "./MarkdownCodeBlock.js";
import type { Components } from "streamdown";
import remarkGfm from "remark-gfm";
export type MarkdownLink = ComponentType<{
  href?: string | undefined;
  children?: ReactNode;
}>;

/** Presentation URL budget only. Hosts retain final navigation authority. */
export function safeWebLink(value: string | undefined): string | null {
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
function InertLink({
  children,
}: {
  href?: string | undefined;
  children?: ReactNode;
}) {
  return <span className="markdown-link-inert">{children}</span>;
}

export interface MarkdownContentProps {
  content: string;
  className?: string;
  linkComponent?: MarkdownLink | undefined;
  isStreaming?: boolean | undefined;
}

interface MarkdownAstNode {
  type: string;
  children?: MarkdownAstNode[];
  depth?: number;
  value?: string;
}

interface MarkdownAstRoot extends MarkdownAstNode {
  children: MarkdownAstNode[];
}

interface MarkdownFile {
  value?: unknown;
}

export const MAX_MARKDOWN_CHARACTERS = 16_384;
export const MAX_MARKDOWN_AST_NODES = 1_000;
const MAX_MARKDOWN_AST_DEPTH = 32;
const STRUCTURE_FALLBACK_NOTICE =
  "Complex response shown as plain text for performance and safety.";

const ALLOWED_ELEMENTS = [
  "a",
  "blockquote",
  "br",
  "code",
  "del",
  "em",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "hr",
  "img",
  "input",
  "li",
  "ol",
  "p",
  "pre",
  "strong",
  "table",
  "tbody",
  "td",
  "th",
  "thead",
  "tr",
  "ul",
] as const;
type MarkdownComponents = Pick<Components, (typeof ALLOWED_ELEMENTS)[number]>;
const REHYPE_PLUGINS = [rehypeSanitize];
// Presentation-only ordinals identify multiple scroll regions without speaking
// framework IDs. Allocated once per mount; no transport/state authority lives here.
let nextScrollRegion = 1;

// Remote images remain blocked. Links become explicit native-browser actions
// only when a browser controller is available; never privileged-view navigation.
// Preserve the validated spelling for native checks against saved instructions.
const safeDestination = (url: string, key: string) =>
  key === "href" && safeWebLink(url) ? url : null;

function BlockedImage({ alt }: Pick<ComponentPropsWithoutRef<"img">, "alt">) {
  return (
    <span className="markdown-image-blocked" role="note">
      Image omitted{alt === undefined || alt === "" ? "" : `: ${alt}`}
    </span>
  );
}

function MarkdownTable({
  children,
}: Pick<ComponentPropsWithoutRef<"table">, "children">) {
  const labelId = useId();
  const [ordinal] = useState(() => nextScrollRegion++);
  function text(node: ReactNode): string {
    if (typeof node === "string" || typeof node === "number")
      return String(node);
    if (isValidElement<{ children?: ReactNode }>(node))
      return Children.toArray(node.props.children).map(text).join(" ");
    return "";
  }
  const heading = Children.toArray(children).find(
    (node) => isValidElement(node) && node.type === "thead",
  );
  const columns = text(heading).trim().slice(0, 240);
  return (
    <div
      className="markdown-table-scroll"
      role="region"
      aria-labelledby={labelId}
      tabIndex={0}
    >
      <span id={labelId} className="shared-sr-only">
        Scrollable Markdown table {ordinal}
        {columns ? `: ${columns}` : ""}
      </span>
      <table>{children}</table>
    </div>
  );
}

function replaceWithPlainText(tree: MarkdownAstRoot, file: MarkdownFile): void {
  const content = typeof file.value === "string" ? file.value : "";
  tree.children = [
    {
      type: "blockquote",
      children: [
        {
          type: "paragraph",
          children: [{ type: "text", value: STRUCTURE_FALLBACK_NOTICE }],
        },
      ],
    },
    { type: "code", value: content },
  ];
}

/**
 * Caps the eventual React tree and keeps response headings beneath the
 * assistant article heading. The character cap separately bounds parser work.
 */
function enforceStructureBudget() {
  return (tree: MarkdownAstRoot, file: MarkdownFile): void => {
    let nodeCount = 0;
    let shallowestHeading = 7;
    const stack = [{ node: tree as MarkdownAstNode, depth: 0 }];

    while (stack.length > 0) {
      const current = stack.pop();
      if (current === undefined) {
        break;
      }

      nodeCount += 1;
      if (
        nodeCount > MAX_MARKDOWN_AST_NODES ||
        current.depth > MAX_MARKDOWN_AST_DEPTH
      ) {
        replaceWithPlainText(tree, file);
        return;
      }

      if (
        current.node.type === "heading" &&
        typeof current.node.depth === "number"
      ) {
        shallowestHeading = Math.min(shallowestHeading, current.node.depth);
      }

      // Streamdown turns raw HTML into escaped text when rehype-raw is absent.
      // Omit it before that conversion, matching the shared renderer contract.
      if (current.node.children)
        current.node.children = current.node.children.filter(
          (child) => child.type !== "html",
        );
      for (const child of current.node.children ?? []) {
        stack.push({ node: child, depth: current.depth + 1 });
      }
    }

    if (shallowestHeading > 6) {
      return;
    }

    const headingStack = [tree as MarkdownAstNode];
    while (headingStack.length > 0) {
      const node = headingStack.pop();
      if (node === undefined) {
        break;
      }
      if (node.type === "heading" && typeof node.depth === "number") {
        node.depth = Math.min(6, 4 + node.depth - shallowestHeading);
      }
      headingStack.push(...(node.children ?? []));
    }
  };
}

const REMARK_PLUGINS = [remarkGfm, enforceStructureBudget];

function tableAlignment(value: string | undefined): string | undefined {
  return value === "left" || value === "center" || value === "right"
    ? value
    : undefined;
}

const MARKDOWN_COMPONENTS: MarkdownComponents = {
  a: InertLink,
  img: BlockedImage,
  pre: MarkdownCodeBlock,
  code: ({ children, className }) => (
    <code className={className}>{children}</code>
  ),
  p: ({ children }) => <p>{children}</p>,
  strong: ({ children }) => <strong>{children}</strong>,
  em: ({ children }) => <em>{children}</em>,
  del: ({ children }) => <del>{children}</del>,
  ul: ({ children, className }) => <ul className={className}>{children}</ul>,
  ol: ({ children, start }) => <ol start={start}>{children}</ol>,
  li: ({ children, className }) => <li className={className}>{children}</li>,
  blockquote: ({ children }) => <blockquote>{children}</blockquote>,
  h1: ({ children }) => <h1>{children}</h1>,
  h2: ({ children }) => <h2>{children}</h2>,
  h3: ({ children }) => <h3>{children}</h3>,
  h4: ({ children }) => <h4>{children}</h4>,
  h5: ({ children }) => <h5>{children}</h5>,
  h6: ({ children }) => <h6>{children}</h6>,
  hr: () => <hr />,
  thead: ({ children }) => <thead>{children}</thead>,
  tbody: ({ children }) => <tbody>{children}</tbody>,
  tr: ({ children }) => <tr>{children}</tr>,
  th: ({ children, style }) => (
    <th data-align={tableAlignment(style?.textAlign)}>{children}</th>
  ),
  td: ({ children, style }) => (
    <td data-align={tableAlignment(style?.textAlign)}>{children}</td>
  ),
  table: MarkdownTable,
  input: ({ checked }) => (
    <input
      type="checkbox"
      checked={checked}
      disabled
      tabIndex={-1}
      readOnly
      aria-label={checked ? "Completed task" : "Incomplete task"}
    />
  ),
};

// A single bounded block preserves the message-wide AST budget and heading levels.
// Completed messages keep literal Markdown; active messages repair incomplete tokens.
const wholeMessageBlock = (content: string): string[] => [content];
const NO_LINK_MODAL = { enabled: false };

function joinClasses(...classes: Array<string | undefined>): string {
  return classes
    .filter((value) => value !== undefined && value !== "")
    .join(" ");
}

/**
 * Renders model-authored Markdown without interpreting HTML, fetching remote
 * media, or allowing model output to navigate the privileged desktop webview.
 */
function MarkdownContentView({
  content,
  className,
  linkComponent,
  isStreaming = false,
}: MarkdownContentProps): ReactNode {
  const classes = joinClasses("shared-markdown", "markdown-content", className);
  const components = useMemo(
    () => ({
      ...MARKDOWN_COMPONENTS,
      a: linkComponent ?? InertLink,
      pre: ({ children }: ComponentPropsWithoutRef<"pre">) => (
        <MarkdownCodeBlock isStreaming={isStreaming}>
          {children}
        </MarkdownCodeBlock>
      ),
    }),
    [linkComponent, isStreaming],
  );

  if (content.length > MAX_MARKDOWN_CHARACTERS) {
    return (
      <div className={joinClasses(classes, "markdown-content-fallback")}>
        <p className="markdown-render-notice">
          Large response shown as plain text for performance and safety.
        </p>
        <div className="markdown-plain-text preserve-lines">{content}</div>
      </div>
    );
  }

  return (
    <div className={classes}>
      <Streamdown
        prefix="ui"
        mode="streaming"
        parseMarkdownIntoBlocksFn={wholeMessageBlock}
        parseIncompleteMarkdown={isStreaming}
        isAnimating={isStreaming}
        controls={false}
        linkSafety={NO_LINK_MODAL}
        allowedElements={ALLOWED_ELEMENTS}
        // Upstream's arbitrary-tag index signature conflicts with its typed
        // intrinsic props. This map contains only our allowlisted HTML tags.
        components={components as unknown as Components}
        rehypePlugins={REHYPE_PLUGINS}
        remarkPlugins={REMARK_PLUGINS}
        skipHtml
        urlTransform={safeDestination}
      >
        {content}
      </Streamdown>
    </div>
  );
}

export function markdownContentPropsAreEqual(
  previous: MarkdownContentProps,
  next: MarkdownContentProps,
): boolean {
  return (
    previous.content === next.content &&
    previous.className === next.className &&
    previous.linkComponent === next.linkComponent &&
    previous.isStreaming === next.isStreaming
  );
}

export const MarkdownContent = /* @__PURE__ */ memo(
  MarkdownContentView,
  markdownContentPropsAreEqual,
);
MarkdownContent.displayName = "MarkdownContent";

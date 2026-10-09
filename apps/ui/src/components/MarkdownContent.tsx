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
import ReactMarkdown from "react-markdown";
import type { Components, UrlTransform } from "react-markdown";
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
];
const REHYPE_PLUGINS = [rehypeSanitize];
// Presentation-only ordinals identify multiple scroll regions without speaking
// framework IDs. Allocated once per mount; no transport/state authority lives here.
let nextScrollRegion = 1;

// Remote images remain blocked. Links become explicit native-browser actions
// only when a browser controller is available; never privileged-view navigation.
// Preserve the validated spelling for native checks against saved instructions.
const safeDestination: UrlTransform = (url, key) =>
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

function MarkdownPre({
  children,
}: Pick<ComponentPropsWithoutRef<"pre">, "children">) {
  const [ordinal] = useState(() => nextScrollRegion++);
  return (
    <pre
      role="region"
      aria-label={`Scrollable Markdown code block ${ordinal}`}
      tabIndex={0}
    >
      {children}
    </pre>
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

const MARKDOWN_COMPONENTS: Components = {
  a: InertLink,
  img: BlockedImage,
  pre: MarkdownPre,
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
}: MarkdownContentProps): ReactNode {
  const classes = joinClasses("shared-markdown", "markdown-content", className);
  const components = useMemo(
    () =>
      linkComponent
        ? { ...MARKDOWN_COMPONENTS, a: linkComponent }
        : MARKDOWN_COMPONENTS,
    [linkComponent],
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
      <ReactMarkdown
        allowedElements={ALLOWED_ELEMENTS}
        components={components}
        rehypePlugins={REHYPE_PLUGINS}
        remarkPlugins={REMARK_PLUGINS}
        skipHtml
        urlTransform={safeDestination}
      >
        {content}
      </ReactMarkdown>
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
    previous.linkComponent === next.linkComponent
  );
}

export const MarkdownContent = /* @__PURE__ */ memo(
  MarkdownContentView,
  markdownContentPropsAreEqual,
);
MarkdownContent.displayName = "MarkdownContent";

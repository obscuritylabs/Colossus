import { memo } from "react";
import type { ComponentPropsWithoutRef, ReactNode } from "react";
import rehypeSanitize from "rehype-sanitize";
import ReactMarkdown from "react-markdown";
import type { Components, UrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";

interface MarkdownContentProps {
  content: string;
  className?: string;
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

// The privileged webview must never navigate to model-authored destinations.
// A future native URL opener can replace this with an explicit allowlisted flow.
const removeDestination: UrlTransform = () => null;

function InertLink({
  children,
}: Pick<ComponentPropsWithoutRef<"a">, "children">) {
  return <span className="markdown-link-inert">{children}</span>;
}

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
  return (
    <div
      className="markdown-table-scroll"
      role="region"
      aria-label="Scrollable Markdown table"
      tabIndex={0}
    >
      <table>{children}</table>
    </div>
  );
}

function MarkdownPre({
  children,
}: Pick<ComponentPropsWithoutRef<"pre">, "children">) {
  return (
    <pre role="region" aria-label="Scrollable Markdown code block" tabIndex={0}>
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
    <input type="checkbox" checked={checked} disabled tabIndex={-1} readOnly />
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
}: MarkdownContentProps): ReactNode {
  const classes = joinClasses("markdown-content", className);

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
        components={MARKDOWN_COMPONENTS}
        rehypePlugins={REHYPE_PLUGINS}
        remarkPlugins={REMARK_PLUGINS}
        skipHtml
        urlTransform={removeDestination}
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
    previous.content === next.content && previous.className === next.className
  );
}

export const MarkdownContent = memo(
  MarkdownContentView,
  markdownContentPropsAreEqual,
);
MarkdownContent.displayName = "MarkdownContent";

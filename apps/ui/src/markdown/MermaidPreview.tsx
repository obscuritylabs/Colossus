import { createElement, useEffect, useId, useState } from "react";
import type { ReactNode } from "react";
import { Button } from "../components/ui/button.js";
import {
  canPreviewMermaid,
  sanitizeMermaidSvg,
  type MermaidSvgNode,
} from "./mermaid-security";

interface MermaidPreviewProps {
  source: string;
  isStreaming?: boolean | undefined;
}

const REACT_ATTRIBUTES: Record<string, string> = {
  "stroke-width": "strokeWidth",
  "stroke-dasharray": "strokeDasharray",
  "marker-start": "markerStart",
  "marker-mid": "markerMid",
  "marker-end": "markerEnd",
  "text-anchor": "textAnchor",
  "dominant-baseline": "dominantBaseline",
};
let renderQueue: Promise<unknown> = Promise.resolve();

function renderTree(node: MermaidSvgNode, key: string): ReactNode {
  const properties = Object.fromEntries(
    Object.entries(node.attributes).map(([name, value]) => [
      REACT_ATTRIBUTES[name] ?? name,
      value,
    ]),
  );
  const size = node.attributes.viewBox
    ?.trim()
    .split(/[\s,]+/u)
    .map(Number);
  if (
    node.tag === "svg" &&
    size?.length === 4 &&
    size[2]! > 0 &&
    size[3]! > 0
  ) {
    properties.width = String(size[2]);
    properties.height = String(size[3]);
  }
  return createElement(
    node.tag,
    {
      ...properties,
      key,
      ...(node.tag === "svg"
        ? {
            className: "markdown-mermaid-diagram",
            role: "img",
            "aria-label": "Mermaid diagram",
            focusable: false,
          }
        : {}),
    },
    node.children.map((child, index) =>
      typeof child === "string" ? child : renderTree(child, `${key}-${index}`),
    ),
  );
}

export function MermaidPreview({
  source,
  isStreaming = false,
}: MermaidPreviewProps): ReactNode {
  const id = `diagram-${useId().replace(/[^\w-]/gu, "")}`;
  const [result, setResult] = useState<{
    source: string;
    tree: MermaidSvgNode | null;
  } | null>(null);
  const [actualSize, setActualSize] = useState(false);
  const permitted = canPreviewMermaid(source);
  useEffect(() => {
    if (!permitted || isStreaming) return;
    let active = true;
    const pending = renderQueue.then(async () => {
      if (!active) return;
      const { default: mermaid } = await import("mermaid");
      if (!active) return;
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        htmlLabels: false,
        suppressErrorRendering: true,
        maxTextSize: 6_000,
        maxEdges: 100,
        theme: "base",
        fontFamily:
          getComputedStyle(document.documentElement)
            .getPropertyValue("--font-sans")
            .trim() || "inherit",
        secure: [
          "secure",
          "securityLevel",
          "startOnLoad",
          "maxTextSize",
          "maxEdges",
          "suppressErrorRendering",
          "htmlLabels",
          "theme",
          "fontFamily",
          "themeCSS",
        ],
      });
      try {
        const { svg } = await mermaid.render(id, source);
        const tree = sanitizeMermaidSvg(svg, id, new DOMParser());
        if (active) setResult({ source, tree });
      } catch {
        // Diagram source is untrusted; only a fixed presentation error is released.
        if (active) setResult({ source, tree: null });
      }
    });
    renderQueue = pending.catch(() => {
      if (active) setResult({ source, tree: null });
    });
    return () => {
      active = false;
    };
  }, [id, source, permitted, isStreaming]);

  if (!permitted)
    return (
      <p className="markdown-mermaid-notice" role="note">
        Diagram preview unavailable for this source. View the code instead.
      </p>
    );
  if (isStreaming)
    return (
      <p className="markdown-mermaid-notice" role="status">
        Diagram preview will appear when the code is complete.
      </p>
    );
  if (result?.source !== source)
    return (
      <p className="markdown-mermaid-notice" role="status">
        Rendering diagram…
      </p>
    );
  if (!result.tree)
    return (
      <p className="markdown-mermaid-notice" role="note">
        Unable to preview this diagram. View the code to check its syntax.
      </p>
    );
  return (
    <div className="markdown-mermaid-frame">
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="markdown-mermaid-size-toggle"
        onClick={() => setActualSize(!actualSize)}
      >
        {actualSize ? "Fit diagram" : "Actual size"}
      </Button>
      <div
        className="markdown-mermaid-preview"
        data-size={actualSize ? "actual" : "fit"}
        tabIndex={0}
        role="region"
        aria-label="Scrollable Mermaid diagram"
      >
        {renderTree(result.tree, "root")}
      </div>
    </div>
  );
}

export default MermaidPreview;

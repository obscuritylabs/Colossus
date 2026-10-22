/** A deliberately small, offline diagram surface for model-authored content. */
export const MAX_MERMAID_CHARACTERS = 6_000;
const MAX_SVG_CHARACTERS = 240_000;
const MAX_SVG_NODES = 2_000;
const MAX_SVG_DEPTH = 32;

const DIAGRAM_START =
  /^(?:(?:flowchart|graph)\s+(?:TB|TD|BT|RL|LR)\b|sequenceDiagram\b|stateDiagram(?:-v2)?\b|classDiagram\b|erDiagram\b|gantt\b|pie\b)/u;
const UNSAFE_SOURCE = [
  /%%\s*\{/u,
  /(?:^|[\n;])\s*(?:click|style|classDef|linkStyle|link|links|linkClass|callback)\b/iu,
  /(?:https?|javascript|data|file|ftp|blob|tauri|vscode):/iu,
  /\burl\s*\(/iu,
  /\/\//u,
  /<(?:[A-Za-z!/?])/u,
  /(?:&|#)(?:#?(?:x[\da-f]+|\d+)|#?[a-z][\da-z]*);/iu,
  /@\s*\{/u,
  /:::/u,
  /\b(?:fa[bsrl]?|mdi|logos|iconify):/iu,
  /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u,
];

export function canPreviewMermaid(source: string): boolean {
  const normalized = source.replace(/\r\n?/gu, "\n").trim();
  return (
    normalized.length > 0 &&
    source.length <= MAX_MERMAID_CHARACTERS &&
    normalized.split(/[\n;]/u).length <= 200 &&
    DIAGRAM_START.test(normalized) &&
    !UNSAFE_SOURCE.some((pattern) => pattern.test(normalized))
  );
}

export interface MermaidSvgNode {
  tag: string;
  attributes: Record<string, string>;
  children: Array<MermaidSvgNode | string>;
}

const ALLOWED_TAGS = new Set([
  "svg",
  "g",
  "defs",
  "marker",
  "path",
  "rect",
  "circle",
  "ellipse",
  "line",
  "polyline",
  "polygon",
  "text",
  "tspan",
  "title",
  "desc",
]);
const NUMERIC_ATTRIBUTES = new Set([
  "x",
  "y",
  "x1",
  "y1",
  "x2",
  "y2",
  "cx",
  "cy",
  "r",
  "rx",
  "ry",
  "width",
  "height",
  "refX",
  "refY",
  "markerWidth",
  "markerHeight",
  "stroke-width",
  "stroke-dasharray",
  "textLength",
]);
const NUMBER_LIST = /^[-+\deE.,\s]+$/u;
const ID = /^[A-Za-z_][\w.-]{0,160}$/u;
const LOCAL_REFERENCE = /^url\(#([A-Za-z_][\w.-]{0,160})\)$/u;

function safeNumbers(value: string): boolean {
  if (value.length === 0 || value.length > 20_000 || !NUMBER_LIST.test(value))
    return false;
  return value
    .trim()
    .split(/[\s,]+/u)
    .every(
      (part) =>
        Number.isFinite(Number(part)) && Math.abs(Number(part)) <= 1_000_000,
    );
}

function safeTransform(value: string): boolean {
  if (value.length > 1_000) return false;
  const operations = value.matchAll(
    /(?:matrix|translate|scale|rotate|skewX|skewY)\(([^)]+)\)/gu,
  );
  let remainder = value;
  for (const operation of operations) {
    if (!safeNumbers(operation[1] ?? "")) return false;
    remainder = remainder.replace(operation[0], "");
  }
  return remainder.trim() === "";
}

/**
 * Copies geometry and text into an inert tree. Generated SVG is never inserted
 * as HTML, and its styles, media, links, events and arbitrary classes are dropped.
 */
export function sanitizeMermaidSvg(
  source: string,
  prefix: string,
  parser: Pick<DOMParser, "parseFromString">,
): MermaidSvgNode | null {
  if (
    source.length > MAX_SVG_CHARACTERS ||
    /<!DOCTYPE|<!ENTITY/iu.test(source) ||
    !/^[A-Za-z][\w-]{0,80}$/u.test(prefix)
  )
    return null;
  const document = parser.parseFromString(source, "image/svg+xml");
  const root = document.documentElement;
  if (root.localName !== "svg" || document.querySelector("parsererror"))
    return null;

  const ids = new Map<string, string>();
  const stack: Array<{ element: Element; depth: number }> = [
    { element: root, depth: 0 },
  ];
  let count = 0;
  while (stack.length > 0) {
    const current = stack.pop();
    if (!current) break;
    if (++count > MAX_SVG_NODES || current.depth > MAX_SVG_DEPTH) return null;
    const original = current.element.getAttribute("id");
    if (original && ID.test(original) && !ids.has(original))
      ids.set(original, `${prefix}-${ids.size}`);
    for (const child of current.element.children)
      stack.push({ element: child, depth: current.depth + 1 });
  }

  function copy(element: Element): MermaidSvgNode | null {
    const tag = element.localName;
    if (
      !ALLOWED_TAGS.has(tag) ||
      element.namespaceURI !== "http://www.w3.org/2000/svg"
    )
      return null;
    const attributes: Record<string, string> = {};
    const originalId = element.getAttribute("id");
    if (originalId && ids.has(originalId)) attributes.id = ids.get(originalId)!;
    for (const attribute of element.attributes) {
      const { name, value } = attribute;
      if (NUMERIC_ATTRIBUTES.has(name) && safeNumbers(value))
        attributes[name] = value;
      else if (
        (name === "dx" || name === "dy") &&
        /^[-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:em)?$/u.test(value) &&
        Math.abs(Number.parseFloat(value)) <= 1_000
      )
        attributes[name] = value;
      else if (
        name === "viewBox" &&
        safeNumbers(value) &&
        value.trim().split(/[\s,]+/u).length === 4
      )
        attributes[name] = value;
      else if (name === "points" && safeNumbers(value))
        attributes[name] = value;
      else if (
        name === "d" &&
        value.length <= 80_000 &&
        /^[MmZzLlHhVvCcSsQqTtAa\dEe.,+\-\s]+$/u.test(value)
      )
        attributes[name] = value;
      else if (name === "transform" && safeTransform(value))
        attributes[name] = value;
      else if (
        (name === "marker-start" ||
          name === "marker-mid" ||
          name === "marker-end") &&
        LOCAL_REFERENCE.test(value)
      ) {
        const reference = LOCAL_REFERENCE.exec(value)?.[1];
        const replacement = reference ? ids.get(reference) : undefined;
        if (replacement) attributes[name] = `url(#${replacement})`;
      } else if (
        name === "orient" &&
        (value === "auto" ||
          value === "auto-start-reverse" ||
          safeNumbers(value))
      )
        attributes[name] = value;
      else if (
        name === "markerUnits" &&
        (value === "strokeWidth" || value === "userSpaceOnUse")
      )
        attributes[name] = value;
      else if (name === "text-anchor" && /^(?:start|middle|end)$/u.test(value))
        attributes[name] = value;
      else if (
        name === "dominant-baseline" &&
        /^(?:auto|middle|central|hanging|alphabetic)$/u.test(value)
      )
        attributes[name] = value;
      else if ((name === "fill" || name === "stroke") && value === "none")
        attributes[name] = value;
      else if (
        name === "preserveAspectRatio" &&
        /^(?:none|x(?:Min|Mid|Max)Y(?:Min|Mid|Max)(?:\s+(?:meet|slice))?)$/u.test(
          value,
        )
      )
        attributes[name] = value;
    }
    return {
      tag,
      attributes,
      children: Array.from(element.childNodes).flatMap<MermaidSvgNode | string>(
        (child) => {
          if (child.nodeType === 3)
            return tag === "text" ||
              tag === "tspan" ||
              tag === "title" ||
              tag === "desc"
              ? [(child.textContent ?? "").slice(0, 8_000)]
              : [];
          if (child.nodeType !== 1) return [];
          const result = copy(child as Element);
          return result ? [result] : [];
        },
      ),
    };
  }
  return copy(root);
}

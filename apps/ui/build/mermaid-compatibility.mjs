import { readFile } from "node:fs/promises";

const CORE_REGISTRATION =
  /  if \(true\) \{\n    registerLazyLoadedDiagrams\(detector_default2, detector_default4, architectureDetector_default\);\n  \}\n  registerLazyLoadedDiagrams\(\n[\s\S]+?\n    usecase\n  \);/u;
const OWNED_REGISTRATION = `  registerLazyLoadedDiagrams(
    flowDetector_v2_default,
    sequenceDetector_default,
    stateDetector_V2_default,
    classDetector_V2_default,
    erDetector_default,
    ganttDetector_default,
    pie
  );`;
const INLINE_STYLES = `  const style1 = document.createElement("style");
  style1.innerHTML = rules;
  svg.insertBefore(style1, firstChild);`;
const LAYOUT_REGISTRATION = /  registerLayoutLoaders\(\[\n[\s\S]+?\n  \]\);/u;
const DAGRE_REGISTRATION = `  registerLayoutLoaders([
    {
      name: "dagre",
      loader: /* @__PURE__ */ __name(async () => await import("./dagre-NK4GIZVX.mjs"), "loader")
    }
  ]);`;
const DIAGRAM_SECTIONS = new Set([
  "flowchart/flowDetector-v2.ts",
  "sequence/sequenceDetector.ts",
  "state/stateDetector-V2.ts",
  "class/classDetector-V2.ts",
  "er/erDetector.ts",
  "gantt/ganttDetector.ts",
  "pie/pieDetector.ts",
  "error/errorRenderer.ts",
  "error/errorDiagram.ts",
]);

function replaceOnce(source, expected, replacement, purpose) {
  const matches =
    typeof expected === "string"
      ? source.split(expected).length - 1
      : Array.from(source.matchAll(new RegExp(expected.source, "gu"))).length;
  if (matches !== 1)
    throw new Error(
      `Pinned Mermaid12.2.0 ${purpose} shape changed; review its shared renderer adaptation.`,
    );
  return source.replace(expected, replacement);
}

/** Keep only reviewed offline diagram grammars and omit upstream inline styling. */
export function patchMermaidForColossus(source, id) {
  const path = id.split("?")[0].replaceAll("\\", "/");
  if (!path.includes("/mermaid/dist/") || !path.endsWith(".mjs")) return null;
  let code = source;
  if (path.endsWith("/mermaid/dist/mermaid.core.mjs")) {
    if (!source.includes('"12.2.0"'))
      throw new Error(
        "Review the shared renderer adaptation before changing Mermaid12.2.0.",
      );
    code = replaceOnce(
      code,
      CORE_REGISTRATION,
      OWNED_REGISTRATION,
      "diagram registration",
    );
    code = replaceOnce(
      code,
      INLINE_STYLES,
      "  // Colossus styles the inert SVG through its shared stylesheet.",
      "inline stylesheet",
    );
    // Remove unused detector bodies too: bundlers discover their dynamic import
    // chunks before dead-code elimination, so pruning registration alone still
    // emits every unsupported diagram and its large layout dependencies.
    code = code.replace(
      /\/\/ src\/diagrams\/([^\n]+)\n[\s\S]+?(?=\n\/\/ src\/|$)/gu,
      (section, name) => (DIAGRAM_SECTIONS.has(name) ? section : ""),
    );
  }
  if (source.includes("// src/rendering-util/render.ts")) {
    code = replaceOnce(
      code,
      LAYOUT_REGISTRATION,
      DAGRE_REGISTRATION,
      "layout registration",
    );
    code = replaceOnce(
      code,
      /var elkLayoutLoaders = [\s\S]+?\nvar registerDefaultLayoutLoaders/u,
      "var registerDefaultLayoutLoaders",
      "unsupported layout loader",
    );
  }
  // These are Mermaid's own generated presentation styles. Retaining their
  // values as inert data avoids CSP violations while keeping internal getters
  // coherent. No policy, sanitizer, URL restriction or native boundary changes.
  code = code.replace(
    /\.(attr|setAttribute)\((["'])style\2/gu,
    '.$1("data-colossus-omitted-style"',
  );
  code = code.replace(
    /\.style\((["'])([\w-]+)\1/gu,
    '.attr("data-colossus-omitted-$2"',
  );
  code = code.replace(
    /attrs\.set\("style",/gu,
    'attrs.set("data-colossus-omitted-style",',
  );
  code = code.replace(
    "  YAMLException2.prototype.constructor = YAMLException2;",
    '  Object.defineProperty(YAMLException2.prototype, "constructor", { value: YAMLException2, writable: true, configurable: true });',
  );
  return code === source ? null : code;
}

export function mermaidViteCompatibility() {
  return {
    name: "colossus-mermaid-compatibility",
    enforce: "pre",
    transform(source, id) {
      const code = patchMermaidForColossus(source, id);
      return code === null ? null : { code, map: null };
    },
  };
}

export function mermaidEsbuildCompatibility() {
  return {
    name: "colossus-mermaid-compatibility",
    setup(build) {
      build.onLoad(
        { filter: /[\\/]mermaid[\\/]dist[\\/].*\.mjs$/ },
        async ({ path }) => {
          const source = await readFile(path, "utf8");
          const contents = patchMermaidForColossus(source, path);
          return contents === null ? null : { contents, loader: "js" };
        },
      );
    },
  };
}

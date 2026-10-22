import { createHighlighterCore } from "@shikijs/core";
import { createJavaScriptRegexEngine } from "@shikijs/engine-javascript";

type Grammar = (typeof import("@shikijs/langs/python"))["default"];
// Explicit imports keep unused grammars out of hosts without module splitting.
const loaders: Record<string, () => Promise<Grammar>> = {
  bash: () => import("@shikijs/langs/bash").then((m) => m.default),
  c: () => import("@shikijs/langs/c").then((m) => m.default),
  cpp: () => import("@shikijs/langs/cpp").then((m) => m.default),
  css: () => import("@shikijs/langs/css").then((m) => m.default),
  dockerfile: () => import("@shikijs/langs/dockerfile").then((m) => m.default),
  go: () => import("@shikijs/langs/go").then((m) => m.default),
  html: () => import("@shikijs/langs/html").then((m) => m.default),
  java: () => import("@shikijs/langs/java").then((m) => m.default),
  javascript: () => import("@shikijs/langs/javascript").then((m) => m.default),
  json: () => import("@shikijs/langs/json").then((m) => m.default),
  jsx: () => import("@shikijs/langs/jsx").then((m) => m.default),
  markdown: () => import("@shikijs/langs/markdown").then((m) => m.default),
  python: () => import("@shikijs/langs/python").then((m) => m.default),
  rust: () => import("@shikijs/langs/rust").then((m) => m.default),
  sql: () => import("@shikijs/langs/sql").then((m) => m.default),
  toml: () => import("@shikijs/langs/toml").then((m) => m.default),
  tsx: () => import("@shikijs/langs/tsx").then((m) => m.default),
  typescript: () => import("@shikijs/langs/typescript").then((m) => m.default),
  xml: () => import("@shikijs/langs/xml").then((m) => m.default),
  yaml: () => import("@shikijs/langs/yaml").then((m) => m.default),
};

const aliases: Record<string, string> = {
  js: "javascript",
  ts: "typescript",
  py: "python",
  rs: "rust",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  yml: "yaml",
  md: "markdown",
  "c++": "cpp",
  h: "c",
  hpp: "cpp",
  jsonc: "json",
  svg: "xml",
};
export function codeLanguage(language: string): string {
  const value = language.toLowerCase().slice(0, 40);
  return aliases[value] ?? value;
}

// Sentinel colors classify tokens, never become inline styles or visible colors.
// The corresponding classes use shared palette tokens, including high contrast.
const classes: Record<string, string> = {
  "#000001": "syntax-text",
  "#000002": "syntax-comment",
  "#000003": "syntax-keyword",
  "#000004": "syntax-string",
  "#000005": "syntax-constant",
  "#000006": "syntax-function",
  "#000007": "syntax-type",
  "#000008": "syntax-invalid",
};
const highlighter = createHighlighterCore({
  engine: createJavaScriptRegexEngine(),
  langs: [],
  themes: [
    {
      name: "colossus-semantic",
      type: "dark",
      colors: {
        "editor.foreground": "#000001",
        "editor.background": "#000000",
      },
      settings: [
        {
          scope: ["comment", "punctuation.definition.comment"],
          settings: { foreground: "#000002" },
        },
        { scope: ["keyword", "storage"], settings: { foreground: "#000003" } },
        {
          scope: ["string", "markup.inline.raw"],
          settings: { foreground: "#000004" },
        },
        {
          scope: ["constant", "variable.other.constant"],
          settings: { foreground: "#000005" },
        },
        {
          scope: ["entity.name.function", "support.function"],
          settings: { foreground: "#000006" },
        },
        {
          scope: ["entity.name.type", "support.type", "entity.name.tag"],
          settings: { foreground: "#000007" },
        },
        { scope: "invalid", settings: { foreground: "#000008" } },
      ],
    },
  ],
});
const pendingLanguages = new Map<string, Promise<void>>();
export interface CodeToken {
  content: string;
  className: string;
}
export async function highlightCode(
  source: string,
  language: string,
): Promise<CodeToken[][] | null> {
  const normalized = codeLanguage(language);
  const loader = loaders[normalized];
  if (!loader || source.length > 16_384) return null;
  const engine = await highlighter;
  let pending = pendingLanguages.get(normalized);
  if (!pending) {
    pending = loader()
      .then((grammar) => engine.loadLanguage(...grammar))
      .then(() => undefined);
    pendingLanguages.set(normalized, pending);
  }
  await pending;
  const result = engine.codeToTokens(source, {
    lang: normalized,
    theme: "colossus-semantic",
  });
  // Highlighting cannot multiply the bounded Markdown into an unbounded React tree.
  if (result.tokens.reduce((count, line) => count + line.length + 1, 0) > 4000)
    return null;
  return result.tokens.map((line) =>
    line.map((token) => ({
      content: token.content,
      className: classes[token.color?.toLowerCase() ?? ""] ?? "syntax-text",
    })),
  );
}

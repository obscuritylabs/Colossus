import { createHighlighterCore } from "@shikijs/core";
import { createJavaScriptRegexEngine } from "@shikijs/engine-javascript";
import githubDarkDefault from "@shikijs/themes/github-dark-default";
import githubLightDefault from "@shikijs/themes/github-light-default";

import type { ResolvedColorTheme } from "./theme/appearance";

type LanguageInput = (typeof import("@shikijs/langs/rust"))["default"];

const LANGUAGE_LOADERS: Readonly<Record<string, () => Promise<LanguageInput>>> =
  {
    bash: () => import("@shikijs/langs/bash").then(({ default: lang }) => lang),
    c: () => import("@shikijs/langs/c").then(({ default: lang }) => lang),
    cpp: () => import("@shikijs/langs/cpp").then(({ default: lang }) => lang),
    css: () => import("@shikijs/langs/css").then(({ default: lang }) => lang),
    dockerfile: () =>
      import("@shikijs/langs/dockerfile").then(({ default: lang }) => lang),
    go: () => import("@shikijs/langs/go").then(({ default: lang }) => lang),
    html: () => import("@shikijs/langs/html").then(({ default: lang }) => lang),
    java: () => import("@shikijs/langs/java").then(({ default: lang }) => lang),
    javascript: () =>
      import("@shikijs/langs/javascript").then(({ default: lang }) => lang),
    jsx: () => import("@shikijs/langs/jsx").then(({ default: lang }) => lang),
    json: () => import("@shikijs/langs/json").then(({ default: lang }) => lang),
    markdown: () =>
      import("@shikijs/langs/markdown").then(({ default: lang }) => lang),
    protobuf: () =>
      import("@shikijs/langs/protobuf").then(({ default: lang }) => lang),
    python: () =>
      import("@shikijs/langs/python").then(({ default: lang }) => lang),
    rust: () => import("@shikijs/langs/rust").then(({ default: lang }) => lang),
    scss: () => import("@shikijs/langs/scss").then(({ default: lang }) => lang),
    sql: () => import("@shikijs/langs/sql").then(({ default: lang }) => lang),
    toml: () => import("@shikijs/langs/toml").then(({ default: lang }) => lang),
    tsx: () => import("@shikijs/langs/tsx").then(({ default: lang }) => lang),
    typescript: () =>
      import("@shikijs/langs/typescript").then(({ default: lang }) => lang),
    xml: () => import("@shikijs/langs/xml").then(({ default: lang }) => lang),
    yaml: () => import("@shikijs/langs/yaml").then(({ default: lang }) => lang),
  };

const highlighter = createHighlighterCore({
  themes: [githubDarkDefault, githubLightDefault],
  langs: [],
  engine: createJavaScriptRegexEngine(),
});
const languageLoads = new Map<string, Promise<void>>();

export interface HighlightedToken {
  content: string;
  color: string | undefined;
}

export type HighlightedLine = HighlightedToken[];

export async function highlightSource(
  content: string,
  language: string,
  colorTheme: ResolvedColorTheme,
): Promise<HighlightedLine[]> {
  const loaded = await highlighter;
  const loader = LANGUAGE_LOADERS[language];
  if (loader !== undefined && !loaded.getLoadedLanguages().includes(language)) {
    let pending = languageLoads.get(language);
    if (pending === undefined) {
      pending = loader()
        .then((grammar) => loaded.loadLanguage(...grammar))
        .then(() => undefined);
      languageLoads.set(language, pending);
    }
    await pending;
  }
  const languageToHighlight = loader === undefined ? "plaintext" : language;
  const result = loaded.codeToTokens(content, {
    lang: languageToHighlight,
    theme:
      colorTheme === "light" ? "github-light-default" : "github-dark-default",
  });
  return result.tokens.map((line) =>
    line.map((token) => ({
      content: token.content,
      color: token.color,
    })),
  );
}

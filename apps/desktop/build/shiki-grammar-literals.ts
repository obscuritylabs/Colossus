import type { Plugin } from "vite";

const grammar = /Object\.freeze\(JSON\.parse\(("(?:[^"\\]|\\.)*")\)\)/gu;

function hasPrototypeKey(value: unknown): boolean {
  return (
    value !== null &&
    typeof value === "object" &&
    (Object.hasOwn(value, "__proto__") ||
      Object.values(value).some(hasPrototypeKey))
  );
}

function grammarLiteral(value: unknown): string {
  const counts = new Map<string, number>();
  function countStrings(candidate: unknown): void {
    if (typeof candidate === "string") {
      counts.set(candidate, (counts.get(candidate) ?? 0) + 1);
    } else if (candidate !== null && typeof candidate === "object") {
      Object.values(candidate).forEach(countStrings);
    }
  }
  countStrings(value);
  // Allow for the dictionary entry, index references and wrapper overhead.
  // Short or unique values stay inline rather than growing small grammars.
  const strings = [...counts]
    .filter(
      ([text, count]) =>
        count > 1 && (JSON.stringify(text).length - 8) * (count - 1) > 32,
    )
    .map(([text]) => text);
  if (strings.length === 0) return `Object.freeze(${JSON.stringify(value)})`;
  const indexes = new Map(strings.map((text, index) => [text, index]));
  function literal(candidate: unknown): string {
    if (typeof candidate === "string" && indexes.has(candidate)) {
      return `g[${indexes.get(candidate)}]`;
    }
    if (Array.isArray(candidate))
      return `[${candidate.map(literal).join(",")}]`;
    if (candidate !== null && typeof candidate === "object") {
      return `{${Object.entries(candidate)
        .map(([key, item]) => `${JSON.stringify(key)}:${literal(item)}`)
        .join(",")}}`;
    }
    return JSON.stringify(candidate);
  }
  // The dictionary is local to this expression, preserving module bindings and
  // releasing its temporary array after the frozen grammar has been created.
  return `(()=>{const g=${JSON.stringify(strings)};return Object.freeze(${literal(value)})})()`;
}

/** Compact pinned grammar data without changing values or evaluating strings. */
export function compactGrammarLiterals(source: string): string {
  return source.replace(grammar, (original: string, encoded: string) => {
    const value: unknown = JSON.parse(JSON.parse(encoded) as string);
    // An object literal gives __proto__ special semantics; retain JSON.parse for it.
    return hasPrototypeKey(value) ? original : grammarLiteral(value);
  });
}

export function shikiGrammarLiterals(): Plugin {
  return {
    name: "colossus-shiki-grammar-literals",
    apply: "build",
    enforce: "pre",
    transform(source, id) {
      const path = id.split("?")[0]?.replaceAll("\\", "/") ?? "";
      if (
        !path.includes("/node_modules/@shikijs/langs/dist/") ||
        !path.endsWith(".mjs")
      )
        return null;
      const code = compactGrammarLiterals(source);
      return code === source ? null : { code, map: null };
    },
  };
}

import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const root = fileURLToPath(new URL("../", import.meta.url));
function sources(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory()
      ? sources(join(directory, entry.name))
      : [join(directory, entry.name)],
  );
}
test("shared UI cannot import a host, transport, or runtime authority", () => {
  for (const path of sources(join(root, "src"))) {
    const source = readFileSync(path, "utf8");
    for (const match of source.matchAll(
      /(?:from\s+|import\s*\()(["'])([^"']+)\1/gu,
    )) {
      const specifier = match[2];
      assert.ok(
        specifier.startsWith("./") ||
          specifier.startsWith("../") ||
          [
            "react",
            "react-dom",
            "@tabler/icons-react",
            "@radix-ui/react-slot",
            "@radix-ui/react-collapsible",
            "@radix-ui/react-dialog",
            "@radix-ui/react-tooltip",
            "react-resizable-panels",
            "@xyflow/react",
            "@radix-ui/react-dropdown-menu",
            "@tanstack/react-table",
            "class-variance-authority",
            "clsx",
            "tailwind-merge",
            "react-markdown",
            "rehype-sanitize",
            "remark-gfm",
            "recharts",
          ].includes(specifier),
        `${path}: forbidden dependency ${specifier}`,
      );
      if (specifier.startsWith("."))
        assert.ok(
          join(path, "..", specifier).startsWith(root),
          `${path}: import escapes shared UI`,
        );
    }
    assert.doesNotMatch(
      source,
      /\b(?:acquireVsCodeApi|fetch|WebSocket|localStorage)\b/u,
      `${path}: host services belong in adapters`,
    );
  }
  const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  assert.equal(manifest.private, true);
  for (const app of ["desktop", "vscode", "web"]) {
    const consumer = JSON.parse(
      readFileSync(join(root, "..", app, "package.json"), "utf8"),
    );
    assert.equal(consumer.dependencies["@colossus/ui"], "file:../ui");
    assert.equal(consumer.dependencies.react, manifest.peerDependencies.react);
  }
});

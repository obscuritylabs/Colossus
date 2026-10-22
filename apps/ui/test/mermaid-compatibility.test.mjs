import assert from "node:assert/strict";
import { readFileSync, existsSync, readdirSync } from "node:fs";
import { test } from "node:test";
import { patchMermaidForColossus } from "../build/mermaid-compatibility.mjs";

const consumers = ["desktop", "vscode", "web"];
const packageRoot = consumers
  .map(
    (consumer) =>
      new URL(`../../${consumer}/node_modules/mermaid/`, import.meta.url),
  )
  .find((candidate) => existsSync(new URL("dist/mermaid.core.mjs", candidate)));
assert.ok(
  packageRoot,
  "Install a consumer's pinned shared UI dependencies first.",
);
const core = readFileSync(
  new URL("dist/mermaid.core.mjs", packageRoot),
  "utf8",
);

test("pinned Mermaid renderer omits upstream CSS injection and registers only reviewed diagram types", () => {
  const adapted = patchMermaidForColossus(
    core,
    "/node_modules/mermaid/dist/mermaid.core.mjs",
  );
  assert.ok(adapted);
  assert.match(adapted, /var errorDiagram_default =/u);
  assert.doesNotMatch(
    adapted,
    /const style1 = document\.createElement\("style"\)/u,
  );
  assert.doesNotMatch(adapted, /\.attr\("style"/u);
  const registration = adapted.slice(
    adapted.indexOf("  registerLazyLoadedDiagrams(\n"),
  );
  const block = registration.slice(0, registration.indexOf("\n  );") + 6);
  assert.match(block, /flowDetector_v2_default/u);
  assert.match(block, /sequenceDetector_default/u);
  assert.match(block, /stateDetector_V2_default/u);
  assert.match(block, /classDetector_V2_default/u);
  assert.match(block, /erDetector_default/u);
  assert.match(block, /ganttDetector_default/u);
  assert.match(block, /\bpie\b/u);
  assert.doesNotMatch(block, /architecture|mindmap|elk|usecase/u);
});

test("dependency changes fail closed rather than silently bypassing the renderer adaptation", () => {
  assert.throws(
    () =>
      patchMermaidForColossus(
        core.replaceAll('"12.2.0"', '"12.3.0"'),
        "/node_modules/mermaid/dist/mermaid.core.mjs",
      ),
    /before changing Mermaid/u,
  );
  assert.throws(
    () =>
      patchMermaidForColossus(
        core.replace(
          "svg.insertBefore(style1, firstChild);",
          "svg.appendChild(style1);",
        ),
        "/node_modules/mermaid/dist/mermaid.core.mjs",
      ),
    /inline stylesheet shape changed/u,
  );
  assert.equal(patchMermaidForColossus(core, "/unrelated/module.mjs"), null);
});

test("layout registry omits unsupported heavyweight loaders and variable-key SVG styles stay inert", () => {
  const directory = new URL("dist/chunks/mermaid.core/", packageRoot);
  const sources = readdirSync(directory)
    .filter((name) => name.endsWith(".mjs"))
    .map((name) => ({
      name,
      content: readFileSync(new URL(name, directory), "utf8"),
    }));
  const layout = sources.find(({ content }) =>
    content.includes("// src/rendering-util/render.ts"),
  );
  assert.ok(layout);
  const adaptedLayout = patchMermaidForColossus(
    layout.content,
    `/node_modules/mermaid/dist/chunks/mermaid.core/${layout.name}`,
  );
  assert.ok(adaptedLayout);
  assert.match(adaptedLayout, /import\("\.\/dagre-/u);
  assert.doesNotMatch(
    adaptedLayout,
    /import\("\.\/(?:elk-|cose-bilkent-|swimlanes-)/u,
  );
  const viewbox = sources.find(({ content }) =>
    content.includes("// src/setupGraphViewbox.js"),
  );
  assert.ok(viewbox);
  const adaptedViewbox = patchMermaidForColossus(
    viewbox.content,
    `/node_modules/mermaid/dist/chunks/mermaid.core/${viewbox.name}`,
  );
  assert.match(adaptedViewbox, /attrs\.set\("data-colossus-omitted-style",/u);
  assert.doesNotMatch(adaptedViewbox, /attrs\.set\("style",/u);
});

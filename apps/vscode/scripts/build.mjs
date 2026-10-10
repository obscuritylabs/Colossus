import { build } from "esbuild";
import { compile, optimize } from "@tailwindcss/node";
import { Scanner } from "@tailwindcss/oxide";
import { validateViewContributions } from "./validate-manifest.mjs";
import { cp, mkdir, readdir, rm, readFile, writeFile } from "node:fs/promises";
import { basename, dirname, resolve } from "node:path";

validateViewContributions(JSON.parse(await readFile("package.json", "utf8")));

await rm("dist", { recursive: true, force: true });
await mkdir("dist", { recursive: true });
const sharedStylesheet = resolve("node_modules/@colossus/ui/styles/shadcn.css");
const compiler = await compile(await readFile(sharedStylesheet, "utf8"), {
  from: sharedStylesheet,
  base: dirname(sharedStylesheet),
  onDependency() {},
});
const sourceRoots =
  compiler.root === "none"
    ? []
    : compiler.root === null
      ? [{ base: process.cwd(), pattern: "**/*", negated: false }]
      : [{ ...compiler.root, negated: false }];
const scanner = new Scanner({
  sources: [
    ...sourceRoots,
    ...compiler.sources,
    {
      base: dirname(process.execPath),
      pattern: basename(process.execPath),
      negated: true,
    },
    {
      base: dirname(sharedStylesheet),
      pattern: basename(sharedStylesheet),
      negated: false,
    },
  ],
});
await writeFile(
  "dist/shadcn.css",
  optimize(compiler.build(scanner.scan()), {
    file: sharedStylesheet,
    minify: true,
  }).code,
);
const hostBuild = await build({
  entryPoints: ["src/extension.ts"],
  bundle: true,
  platform: "node",
  target: "node22",
  format: "cjs",
  outfile: "dist/extension.cjs",
  external: ["vscode", "@napi-rs/keyring"],
  sourcemap: true,
  metafile: true,
});
const browserBuild = await build({
  entryPoints: {
    webview: "webview/main.tsx",
    settings: "webview/settings.tsx",
    explorer: "webview/explorer.ts",
    inspector: "webview/inspector.tsx",
  },
  bundle: true,
  platform: "browser",
  target: "es2022",
  format: "iife",
  jsx: "automatic",
  define: { "process.env.NODE_ENV": '"production"' },
  loader: { ".svg": "dataurl" },
  outdir: "dist",
  metafile: true,
});
// The inspector entry imports the shared inbox stylesheet; esbuild emits dist/inspector.css.
const licenses = new Map();
for (const input of [
  ...Object.keys(hostBuild.metafile.inputs),
  ...Object.keys(browserBuild.metafile.inputs),
]) {
  if (!input.startsWith("node_modules/")) continue;
  let directory = dirname(resolve(input));
  while (directory !== process.cwd()) {
    const manifest = await readFile(`${directory}/package.json`, "utf8").catch(
      () => undefined,
    );
    if (manifest) {
      if (!licenses.has(directory)) {
        const info = JSON.parse(manifest);
        const texts = [];
        for (const name of [
          "LICENSE",
          "LICENSE.txt",
          "LICENSE.md",
          "LICENCE",
          "NOTICE",
          "NOTICE.txt",
        ]) {
          const text = await readFile(`${directory}/${name}`, "utf8").catch(
            () => undefined,
          );
          if (text) texts.push(text);
        }
        licenses.set(
          directory,
          `${info.name}@${info.version} (${info.license ?? "see package"})\n${texts.join("\n")}`,
        );
      }
      break;
    }
    directory = dirname(directory);
  }
}
const tabler = JSON.parse(
  await readFile("node_modules/@tabler/icons/package.json", "utf8"),
);
licenses.set(
  "tabler",
  `@tabler/icons@${tabler.version} (MIT)\n${await readFile("node_modules/@tabler/icons/LICENSE", "utf8")}`,
);
licenses.set(
  "shadcn",
  await readFile("node_modules/@colossus/ui/assets/shadcn-LICENSE.txt", "utf8"),
);
await cp(
  "node_modules/@colossus/ui/assets/shadcn-LICENSE.txt",
  "dist/shadcn-LICENSE.txt",
);
await writeFile(
  "dist/THIRD_PARTY_NOTICES.txt",
  [...licenses.values()].join(
    "\n\n----------------------------------------\n\n",
  ),
);
await cp("webview/style.css", "dist/style.css");
await cp("webview/settings.css", "dist/settings.css");
await cp("node_modules/@colossus/ui/styles/select.css", "dist/select.css");
await cp("webview/workspace.css", "dist/workspace.css");
await cp("node_modules/@colossus/ui/styles/theme.css", "dist/theme.css");
await cp("node_modules/@colossus/ui/styles/composer.css", "dist/composer.css");
await cp(
  "node_modules/@colossus/ui/styles/settings.css",
  "dist/settings-frame.css",
);
await mkdir("dist/icons", { recursive: true });
for (const name of [
  "settings",
  "history",
  "plus",
  "refresh",
  "paperclip",
  "arrow-up",
  "square",
  "folder",
  "world",
  "palette",
  "plug-connected",
  "shield",
  "arrow-left",
  "terminal-2",
  "send-2",
  "list-check",
  "messages",
  "activity",
  "layout-sidebar",
]) {
  await cp(
    `node_modules/@tabler/icons/icons/outline/${name}.svg`,
    `dist/icons/${name}.svg`,
  );
}
// Reuse the shipped Desktop assets so the extension keeps the same Colossus identity.
await cp(
  "node_modules/@colossus/ui/assets/colossus-mark.svg",
  "dist/colossus-mark.svg",
);
await mkdir("media", { recursive: true });
await cp("../desktop/src-tauri/icons/icon.png", "media/colossus.png");
// Keep the native credential provider and installed platform binding beside the host bundle.
for (const name of await readdir("node_modules/@napi-rs")) {
  if (name === "keyring" || name.startsWith("keyring-")) {
    await mkdir("dist/node_modules/@napi-rs", { recursive: true });
    await cp(
      `node_modules/@napi-rs/${name}`,
      `dist/node_modules/@napi-rs/${name}`,
      { recursive: true },
    );
  }
}

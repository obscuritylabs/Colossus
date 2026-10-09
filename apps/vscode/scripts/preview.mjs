import { createServer } from "node:http";
import { readFile, readdir } from "node:fs/promises";

const paths = new Map([
  ["/webview.js", ["dist/webview.js", "text/javascript"]],
  ["/explorer.js", ["dist/explorer.js", "text/javascript"]],
  ["/inspector.css", ["dist/inspector.css", "text/css"]],
  ["/inspector.js", ["dist/inspector.js", "text/javascript"]],
  ["/workspace.css", ["dist/workspace.css", "text/css"]],
  ["/theme.css", ["dist/theme.css", "text/css"]],
  ["/shadcn.css", ["dist/shadcn.css", "text/css"]],
  ["/style.css", ["dist/style.css", "text/css"]],
  ["/composer.css", ["dist/composer.css", "text/css"]],
  ["/select.css", ["dist/select.css", "text/css"]],
  ["/settings-frame.css", ["dist/settings-frame.css", "text/css"]],
  ["/settings.js", ["dist/settings.js", "text/javascript"]],
  ["/settings.css", ["dist/settings.css", "text/css"]],
  ["/colossus-mark.svg", ["dist/colossus-mark.svg", "image/svg+xml"]],
]);
for (const name of await readdir("dist/icons"))
  paths.set(`/icons/${name}`, [`dist/icons/${name}`, "image/svg+xml"]);
const server = createServer(async (request, response) => {
  try {
    const path = paths.get(request.url);
    if (path) {
      response.writeHead(200, { "Content-Type": path[1] });
      response.end(await readFile(path[0]));
      return;
    }
    if (!["/", "/settings", "/explorer", "/inspector"].includes(request.url)) {
      response.writeHead(404).end();
      return;
    }
    response.writeHead(200, {
      "Content-Type": "text/html",
      "Content-Security-Policy":
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'none'; base-uri 'none'; form-action 'none'",
    });
    const page = request.url === "/" ? "webview" : request.url.slice(1);
    const css =
      page === "settings" ? "settings" : page === "webview" ? "" : "workspace";
    response.end(
      `<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Colossus ${page} preview</title><link rel="stylesheet" href="/theme.css"><link rel="stylesheet" href="/shadcn.css"><link rel="stylesheet" href="/style.css"><link rel="stylesheet" href="/composer.css"><link rel="stylesheet" href="/select.css">${page === "inspector" ? '<link rel="stylesheet" href="/inspector.css">' : ""}${page === "settings" ? '<link rel="stylesheet" href="/settings-frame.css">' : ""}${css ? `<link rel="stylesheet" href="/${css}.css">` : ""}</head><body data-colossus-mark="/colossus-mark.svg"><div id="app"></div><script src="/${page}.js"></script></body></html>`,
    );
  } catch {
    response.writeHead(500).end();
  }
});
server.listen(4312, "127.0.0.1");

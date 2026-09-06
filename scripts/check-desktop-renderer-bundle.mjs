import { readdir, readFile, stat } from "node:fs/promises";
import { join, relative, resolve } from "node:path";

const desktop = resolve(process.argv[2] ?? "apps/desktop");
const dist = join(desktop, "dist");
const maximumJavaScriptChunkBytes = 700_000;
const maximumRendererBytes = 4_000_000;
const forbiddenFixtureStrings = [
  "fixture-run-desktop-release",
  "fixture-session-operations-studio",
  "fixture-managed-local",
  "Sentinel completed a read-only security pass",
  "Stopped in the UI showcase",
  "nativePluginAcceptance",
  "Test settings value",
  "plugin-test-bridge",
];

async function files(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map((entry) => {
      const path = join(directory, entry.name);
      return entry.isDirectory() ? files(path) : [path];
    }),
  );
  return nested.flat();
}

const emitted = await files(dist);
let total = 0;
const violations = [];
for (const path of emitted) {
  const metadata = await stat(path);
  total += metadata.size;
  if (path.endsWith(".js") && metadata.size > maximumJavaScriptChunkBytes) {
    violations.push(
      `${relative(desktop, path)} is ${metadata.size} bytes; limit is ${maximumJavaScriptChunkBytes}`,
    );
  }
  if (path.endsWith(".js") || path.endsWith(".html")) {
    const source = await readFile(path, "utf8");
    for (const fixture of forbiddenFixtureStrings) {
      if (source.includes(fixture)) {
        violations.push(
          `${relative(desktop, path)} contains development fixture text: ${fixture}`,
        );
      }
    }
  }
}
if (total > maximumRendererBytes) {
  violations.push(
    `renderer output is ${total} bytes; limit is ${maximumRendererBytes}`,
  );
}
if (violations.length > 0) {
  process.stderr.write(`${violations.join("\n")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(
    `Desktop renderer bundle: ${total} bytes (limit ${maximumRendererBytes}).\n`,
  );
}

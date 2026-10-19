import { readdir, readFile, stat } from "node:fs/promises";
import { join, relative, resolve } from "node:path";

const desktop = resolve(process.argv[2] ?? "apps/desktop");
const dist = join(desktop, "dist");
const maximumJavaScriptChunkBytes = 700_000;
// Regular dictation includes a bounded 50 KB allowance; weights stay native.
// Desktop's shared shadcn thread menu adds ~81 KB of Radix menu/focus/positioning
// code in a lazy chunk. Allow 100 KB total growth while retaining the chunk limit.
// Shared automation tables, badges and host-owned navigation retry state add
// about 4 KB; reserve 10 KB across platform-specific bundler output. The 700 KB
// per-chunk limit and production fixture exclusions remain unchanged.
// Shared inbox inspection exceeds the previous ceiling by about 4 KB. Reserve
// 10 KB for this feature across platform bundler output, retaining chunk bounds.
// The shared model popover and discrete reasoning slider reuse the shipped Radix
// foundations. Reserve 40 KB for these controls and their token-based styles;
// retain the per-chunk bound and production fixture exclusions.
// Inbox reading states, readable delivery receipts and inline browser screenshots
// add 5.4 KB measured against the previous build. Reserve 10 KB for this UI change;
// keep per-chunk bounds and development fixture exclusions intact.
// Computer access settings, released screenshot previews, and explicit browser
// expiry recovery add 3.1 KB measured. Reserve 5 KB for platform bundler variance;
// retain the per-chunk limit and production fixture exclusions.
const maximumRendererBytes = 4_225_000;
const forbiddenFixtureStrings = [
  "Offline dictation preview",
  "fixture-run-desktop-release",
  "fixture-session-operations-studio",
  "fixture-message-",
  "fixture-managed-local",
  "Sentinel completed a read-only security pass",
  "Stopped in the UI showcase",
  "nativePluginAcceptance",
  "Test settings value",
  "plugin-test-bridge",
  "approval-test-bridge",
  "nativeCommandApproval",
  "Unsupported setup preview command:",
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

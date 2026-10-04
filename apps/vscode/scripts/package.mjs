import { spawnSync } from "node:child_process";
import { validateViewContributions } from "./validate-manifest.mjs";
import { cp, mkdir, mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { join } from "node:path";

const manifest = JSON.parse(await readFile("package.json", "utf8"));
validateViewContributions(manifest);

const target = process.argv[2] ?? `${process.platform}-${process.arch}`;
if (
  ![
    "linux-x64",
    "linux-arm64",
    "darwin-x64",
    "darwin-arm64",
    "win32-x64",
    "win32-arm64",
  ].includes(target)
)
  throw new Error("Unsupported VSIX platform");
const binding = `@napi-rs/keyring-${target}${target.startsWith("linux-") ? "-gnu" : target.startsWith("win32-") ? "-msvc" : ""}`;
const lock = JSON.parse(await readFile("package-lock.json", "utf8"));
const entry = lock.packages[`node_modules/${binding}`];
if (!entry?.resolved || !entry.integrity?.startsWith("sha512-"))
  throw new Error(
    "Native keyring binding is missing from the dependency lock.",
  );
const temporary = await mkdtemp(join(tmpdir(), "colossus-vsix-"));
try {
  const packed = spawnSync(
    process.platform === "win32" ? "npm.cmd" : "npm",
    [
      "pack",
      entry.resolved,
      "--ignore-scripts",
      "--json",
      "--pack-destination",
      temporary,
    ],
    { encoding: "utf8" },
  );
  if (packed.status !== 0)
    throw new Error("Could not fetch the locked native keyring binding.");
  const filename = JSON.parse(packed.stdout)[0].filename;
  const archive = join(temporary, filename);
  const actual = `sha512-${createHash("sha512")
    .update(await readFile(archive))
    .digest("base64")}`;
  if (actual !== entry.integrity)
    throw new Error("Native keyring archive integrity mismatch.");
  for (const name of await readdir("dist/node_modules/@napi-rs")) {
    if (name.startsWith("keyring-"))
      await rm(`dist/node_modules/@napi-rs/${name}`, {
        recursive: true,
        force: true,
      });
  }
  const destination = `dist/node_modules/${binding}`;
  await rm(destination, { recursive: true, force: true });
  await mkdir(destination, { recursive: true });
  const extracted = spawnSync("tar", [
    "-xzf",
    archive,
    "--strip-components=1",
    "-C",
    destination,
  ]);
  if (extracted.status !== 0)
    throw new Error("Could not stage the native keyring binding.");
  // Keep the dependency's license and wrapper even when cross-packaging from another host.
  await cp(
    "node_modules/@napi-rs/keyring",
    "dist/node_modules/@napi-rs/keyring",
    { recursive: true },
  );
} finally {
  await rm(temporary, { recursive: true, force: true });
}
await mkdir("artifacts", { recursive: true });
const result = spawnSync(
  process.execPath,
  [
    "node_modules/@vscode/vsce/vsce",
    "package",
    "--no-dependencies",
    "--target",
    target,
    "--out",
    `artifacts/colossus-${target}-${manifest.version}.vsix`,
  ],
  { stdio: "inherit" },
);
process.exit(result.status ?? 1);

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { createReadStream } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  writeFile,
  link,
  rm,
} from "node:fs/promises";
import { isAbsolute, join, resolve } from "node:path";
import { assertNativeTarget } from "./native-platform.mjs";
import { open } from "node:fs/promises";

const target = process.argv[2] ?? `${process.platform}-${process.arch}`;
if (!/^(linux|darwin|win32)-(x64|arm64)$/u.test(target))
  throw new Error("Unsupported VSIX platform");
const explicitArtifacts = process.argv[3] === "--cargo-artifacts";
const artifacts = explicitArtifacts ? JSON.parse(process.argv[4]) : undefined;
const source = explicitArtifacts
  ? undefined
  : resolve(process.argv[3] ?? "../../target/debug");
if (
  explicitArtifacts &&
  (!artifacts ||
    typeof artifacts !== "object" ||
    Array.isArray(artifacts) ||
    Object.keys(artifacts).length !== 2 ||
    ["colossus-app-host", "colossus-sidecar"].some(
      (name) =>
        typeof artifacts[name] !== "string" || !isAbsolute(artifacts[name]),
    ))
)
  throw new Error("Invalid Cargo native artifact paths.");
const suffix = target.startsWith("win32-") ? ".exe" : "";
const manifest = {
  version: 1,
  target,
  macos_developer_preview: target.startsWith("darwin-"),
};
const sources = [];
for (const name of ["colossus-app-host", "colossus-sidecar"]) {
  const binary = artifacts ? artifacts[name] : join(source, name + suffix);
  const metadata = await lstat(binary);
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > 512 * 1024 * 1024
  )
    throw new Error("Invalid managed runtime executable");
  const file = await open(binary, "r");
  try {
    const header = Buffer.alloc(Math.min(65536, metadata.size));
    await file.read(header, 0, header.length, 0);
    assertNativeTarget(header, target);
  } finally {
    await file.close();
  }
  sources.push({ name, binary });
}
await mkdir("native", { recursive: true });
for (const name of ["colossus-app-host", "colossus-sidecar"])
  for (const extension of ["", ".exe"])
    await rm(join("native", name + extension), { force: true });
for (const { name, binary } of sources) {
  const destination = join("native", name + suffix);
  if (process.argv[4] === "--hardlink" && process.platform !== "darwin")
    await link(binary, destination);
  else await copyFile(binary, destination);
  await chmod(destination, 0o755);
  if (
    process.platform === "darwin" &&
    target === `${process.platform}-${process.arch}`
  ) {
    const result = spawnSync("/usr/bin/codesign", [
      "--force",
      "--sign",
      "-",
      destination,
    ]);
    if (result.status !== 0)
      throw new Error("Could not sign the macOS Developer Preview runtime.");
    const verified = spawnSync(
      "/usr/bin/codesign",
      ["--verify", "--strict", destination],
      { stdio: "inherit" },
    );
    if (verified.status !== 0)
      throw new Error(
        "Could not verify the macOS Developer Preview signature.",
      );
    // Public code identity diagnostics contain no runtime setup or credentials.
    const identity = spawnSync(
      "/usr/bin/codesign",
      ["--display", "--verbose=4", destination],
      { stdio: "inherit" },
    );
    if (identity.status !== 0)
      throw new Error(
        "Could not inspect the macOS Developer Preview signature.",
      );
  }
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(destination)) hash.update(chunk);
  manifest[name] = hash.digest("hex");
}
await writeFile(
  "native/manifest.json",
  `${JSON.stringify(manifest, null, 2)}\n`,
);

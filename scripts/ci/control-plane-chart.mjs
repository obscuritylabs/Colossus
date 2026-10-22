import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { validateRequest } from "./verify-control-plane-assets.mjs";

export const chartName = "colossus-control-plane-chart";
export function verifyCandidate(candidate, bytes, tag, commit) {
  validateRequest(tag, commit);
  const version = tag.slice(1);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  if (candidate.tag !== tag || candidate.commit !== commit ||
      candidate.version !== version || candidate.name !== chartName ||
      candidate.archive !== `${chartName}-${version}.tgz` || candidate.sha256 !== sha256) {
    throw new Error("Chart candidate does not match the trusted tag, commit, or archive bytes.");
  }
  return sha256;
}
export function verifySource(commit, checkoutCommit, status) {
  if (commit !== checkoutCommit || status.trim()) {
    throw new Error("Chart candidates require the exact clean source checkout.");
  }
}
export function verifyVersions(metadata, cargo, tag) {
  const scalar = (name) => metadata.match(new RegExp(`^${name}: ["']?([^\\s"']+)["']?\\s*$`, "m"))?.[1];
  const version = cargo.split("[workspace.package]")[1]?.split("\n[")[0]?.match(/^version = "([^"]+)"$/m)?.[1];
  if (!version || scalar("name") !== chartName || scalar("version") !== version ||
      scalar("appVersion") !== version || (tag && tag !== `v${version}`)) {
    throw new Error("Chart version, appVersion, and workspace version must match the release tag.");
  }
  return version;
}
async function main() {
  const [mode, directory, tag, commit] = process.argv.slice(2);
  validateRequest(tag, commit);
  const archive = `${chartName}-${tag.slice(1)}.tgz`;
  const bytes = await readFile(join(directory, archive));
  const manifest = join(directory, "candidate.json");
  if (mode === "candidate") {
    verifySource(commit, execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
      execFileSync("git", ["status", "--porcelain=v1", "--untracked-files=normal"], { encoding: "utf8" }));
    verifyVersions(await readFile("deploy/control-plane/chart/Chart.yaml", "utf8"), await readFile("Cargo.toml", "utf8"), tag);
    const candidate = { tag, commit, name: chartName, version: tag.slice(1), archive,
      sha256: createHash("sha256").update(bytes).digest("hex") };
    await writeFile(manifest, `${JSON.stringify(candidate, null, 2)}\n`, { flag: "wx" });
  } else if (mode === "verify") {
    verifyCandidate(JSON.parse(await readFile(manifest, "utf8")), bytes, tag, commit);
  } else { throw new Error("Expected candidate or verify."); }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}

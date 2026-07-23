#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const REQUIRED_PATHS = [
  "LICENSE",
  "README.md",
  "package.json",
  "dist/index.js",
  "dist/index.d.ts",
  "dist/error.js",
  "dist/gen/colossus/api/v1alpha1/agent_run.js",
  "dist/gen/colossus/api/v1alpha1/agent_run.d.ts",
  "dist/gen/google/rpc/status.js",
  "dist/gen/google/rpc/status.d.ts",
];

export function validatePackReport(reports) {
  if (!Array.isArray(reports) || reports.length !== 1) {
    throw new Error("npm pack returned an unexpected report");
  }
  const files = reports[0]?.files;
  if (!Array.isArray(files)) {
    throw new Error("npm pack report is missing its file list");
  }
  const paths = new Set(files.map(({ path }) => path));
  for (const required of REQUIRED_PATHS) {
    if (!paths.has(required)) {
      throw new Error(`TypeScript package is missing ${required}`);
    }
  }
  for (const path of paths) {
    if (path.startsWith("src/") || path.startsWith(".test-dist/")) {
      throw new Error(`TypeScript package contains development source: ${path}`);
    }
  }
}

function main() {
  const output = execFileSync(
    "npm",
    ["pack", "--dry-run", "--json", "--silent"],
    { encoding: "utf8" },
  );
  validatePackReport(JSON.parse(output));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}

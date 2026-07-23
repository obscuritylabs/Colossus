import assert from "node:assert/strict";
import test from "node:test";

import { validatePackReport } from "./check-typescript-package.mjs";

const required = [
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
].map((path) => ({ path }));

test("accepts the intended package surface", () => {
  assert.doesNotThrow(() => validatePackReport([{ files: required }]));
});

test("rejects missing release files and development source", () => {
  assert.throws(
    () => validatePackReport([{ files: required.slice(1) }]),
    /missing LICENSE/,
  );
  assert.throws(
    () =>
      validatePackReport([
        { files: [...required, { path: "src/credential.ts" }] },
      ]),
    /development source/,
  );
});

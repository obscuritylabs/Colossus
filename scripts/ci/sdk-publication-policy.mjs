#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

// Read from the immutable release checkout, not mutable repository settings or
// release notes. A manual recovery request cannot override a Core/Desktop release.
export function authorizeSdkPublication(cargoMetadata, requested) {
  const policy = cargoMetadata?.metadata?.release?.["publish-sdks"];
  if (typeof policy !== "boolean" || typeof requested !== "boolean") {
    throw new Error("SDK publication requires an explicit boolean release policy and request");
  }
  return policy && requested;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (!["true", "false"].includes(process.argv[2])) {
    throw new Error("Expected publication request true or false");
  }
  const cargo = JSON.parse(readFileSync(0, "utf8"));
  console.log(authorizeSdkPublication(cargo, process.argv[2] === "true"));
}

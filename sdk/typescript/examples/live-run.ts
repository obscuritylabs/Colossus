/**
 * Connect the TypeScript SDK using a one-use credential from an anonymous stdin pipe.
 */

import { fstatSync, readFileSync } from "node:fs";

import {
  StaticBearerCredential,
  createSecureGrpcClient,
  parseEndpointDescriptor,
} from "../src/index.js";
import { AgentRunServiceClient } from "../src/gen/colossus/api/v1alpha1/agent_run.js";
import { DeploymentMode } from "../src/gen/colossus/api/v1alpha1/system.js";

import { runPrompt } from "./durable-run.js";

const MAX_CREDENTIAL_BYTES = 761;

function readPipeCredential(): StaticBearerCredential {
  const stdin = fstatSync(0);
  if (!stdin.isFIFO() && !stdin.isSocket()) {
    throw new Error(
      "the live SDK credential must arrive through an anonymous pipe",
    );
  }
  const token = readFileSync(0);
  if (token.length === 0 || token.length > MAX_CREDENTIAL_BYTES) {
    throw new Error("the live SDK credential is invalid");
  }
  return new StaticBearerCredential(token.toString("ascii"));
}

async function main(): Promise<void> {
  const [descriptorPath, certificatePath, instanceId, certificateSha256, prompt] =
    process.argv.slice(2);
  if (
    descriptorPath === undefined ||
    certificatePath === undefined ||
    instanceId === undefined ||
    certificateSha256 === undefined ||
    prompt === undefined
  ) {
    throw new Error(
      "usage: live-run DESCRIPTOR CERTIFICATE INSTANCE_ID CERTIFICATE_SHA256 PROMPT",
    );
  }
  const descriptor = parseEndpointDescriptor(
    readFileSync(descriptorPath, "utf8"),
  );
  const certificate = readFileSync(certificatePath);
  const credential = readPipeCredential();
  const client = await createSecureGrpcClient(
    AgentRunServiceClient,
    descriptor,
    certificate,
    instanceId,
    certificateSha256,
    DeploymentMode.DEPLOYMENT_MODE_SHARED_DAEMON,
    credential,
  );
  try {
    const result = await runPrompt(client, prompt);
    process.stdout.write(`${result.output}\n`);
  } finally {
    client.close();
  }
}

await main();

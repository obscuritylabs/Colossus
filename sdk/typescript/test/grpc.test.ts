import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import * as grpc from "@grpc/grpc-js";

import { StaticBearerCredential } from "../src/credential.js";
import {
  certificateSha256,
  parseEndpointDescriptor,
} from "../src/endpoint.js";
import {
  assertCompatibleServerInfo,
  createSecureGrpcClient,
} from "../src/grpc.js";
import { AgentRunServiceClient } from "../src/gen/colossus/api/v1alpha1/agent_run.js";
import {
  DeploymentMode,
  SystemServiceService,
  type SystemServiceServer,
  type ServerInfo,
} from "../src/gen/colossus/api/v1alpha1/system.js";

const expectedInstanceId = "00000000-0000-4000-8000-000000000001";

function serverInfo(changed: Partial<ServerInfo> = {}): ServerInfo {
  return {
    instanceId: expectedInstanceId,
    serverVersion: "0.9.0",
    apiPackages: ["colossus.api.v1alpha1"],
    deploymentMode: DeploymentMode.DEPLOYMENT_MODE_SHARED_DAEMON,
    capabilities: [],
    limits: [],
    deprecations: [],
    serverTime: undefined,
    ...changed,
  };
}

test("authenticated server info binds live identity, API, and deployment", () => {
  assert.doesNotThrow(() =>
    assertCompatibleServerInfo(
      serverInfo(),
      expectedInstanceId,
      DeploymentMode.DEPLOYMENT_MODE_SHARED_DAEMON,
    ),
  );
  for (const incompatible of [
    serverInfo({ instanceId: "00000000-0000-4000-8000-000000000002" }),
    serverInfo({ apiPackages: ["colossus.api.v2"] }),
    serverInfo({ deploymentMode: DeploymentMode.DEPLOYMENT_MODE_SIDECAR }),
  ]) {
    assert.throws(
      () =>
        assertCompatibleServerInfo(
          incompatible,
          expectedInstanceId,
          DeploymentMode.DEPLOYMENT_MODE_SHARED_DAEMON,
        ),
      /incompatible/u,
    );
  }
});

test("gRPC connectors cannot claim embedded or unspecified deployment", () => {
  for (const deploymentMode of [
    DeploymentMode.DEPLOYMENT_MODE_UNSPECIFIED,
    DeploymentMode.DEPLOYMENT_MODE_EMBEDDED,
  ]) {
    assert.throws(
      () =>
        assertCompatibleServerInfo(
          serverInfo(),
          expectedInstanceId,
          deploymentMode,
        ),
      /deployment mode/u,
    );
  }
});

test("connector verifies pinned TLS, bearer auth, and live server identity", async () => {
  const certificate = readFileSync(
    new URL("../../../testdata/connector-cert.pem", import.meta.url),
  );
  const privateKey = readFileSync(
    new URL("../../../testdata/connector-key.pem", import.meta.url),
  );
  const server = new grpc.Server();
  let authenticatedCalls = 0;
  let applicationClientConstructed = false;
  let applicationChannelRequests = 0;
  let clientExistedDuringCompatibilityCall = false;
  class TrackingAgentRunServiceClient extends AgentRunServiceClient {
    public constructor(
      address: string,
      credentials: grpc.ChannelCredentials,
      options?: grpc.ClientOptions,
    ) {
      super(address, credentials, options);
      applicationClientConstructed = true;
    }

    public override getChannel() {
      applicationChannelRequests += 1;
      return super.getChannel();
    }
  }
  const implementation: SystemServiceServer = {
    getServerInfo(call, callback) {
      assert.deepEqual(call.metadata.get("authorization"), [
        "Bearer connector-test-token",
      ]);
      authenticatedCalls += 1;
      clientExistedDuringCompatibilityCall = applicationClientConstructed;
      callback(null, {
        serverInfo: serverInfo(),
      });
    },
    getReadiness(_call, callback) {
      callback(null, { status: 1, checks: [] });
    },
  };
  server.addService(SystemServiceService, implementation);
  const port = await new Promise<number>((resolve, reject) => {
    server.bindAsync(
      "127.0.0.1:0",
      grpc.ServerCredentials.createSsl(null, [
        { cert_chain: certificate, private_key: privateKey },
      ]),
      (error, boundPort) => {
        if (error !== null) {
          reject(error);
          return;
        }
        resolve(boundPort);
      },
    );
  });
  try {
    const pin = certificateSha256(certificate);
    const descriptor = parseEndpointDescriptor({
      schema_version: 1,
      api_version: "colossus.api.v1alpha1",
      instance_id: expectedInstanceId,
      endpoint: `https://127.0.0.1:${port}`,
      pid: 1,
      certificate_sha256: pin,
    });
    const client = await createSecureGrpcClient(
      TrackingAgentRunServiceClient,
      descriptor,
      certificate,
      expectedInstanceId,
      pin,
      DeploymentMode.DEPLOYMENT_MODE_SHARED_DAEMON,
      new StaticBearerCredential("connector-test-token"),
    );
    assert.equal(authenticatedCalls, 1);
    assert.equal(clientExistedDuringCompatibilityCall, true);
    assert.equal(applicationChannelRequests, 1);
    client.close();

    const wrongLeaf = readFileSync(
      new URL("../../../testdata/leaf.pem", import.meta.url),
    );
    const wrongPin = certificateSha256(wrongLeaf);
    const wrongDescriptor = parseEndpointDescriptor({
      schema_version: 1,
      api_version: "colossus.api.v1alpha1",
      instance_id: expectedInstanceId,
      endpoint: `https://127.0.0.1:${port}`,
      pid: 1,
      certificate_sha256: wrongPin,
    });
    await assert.rejects(() =>
      createSecureGrpcClient(
        AgentRunServiceClient,
        wrongDescriptor,
        wrongLeaf,
        expectedInstanceId,
        wrongPin,
        DeploymentMode.DEPLOYMENT_MODE_SHARED_DAEMON,
        new StaticBearerCredential("wrong-leaf-token"),
      ),
    );
    assert.equal(authenticatedCalls, 1);
  } finally {
    await new Promise<void>((resolve) => server.tryShutdown(() => resolve()));
  }
});

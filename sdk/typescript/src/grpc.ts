import { createHash } from "node:crypto";
import { checkServerIdentity } from "node:tls";

import * as grpc from "@grpc/grpc-js";

import { StaticBearerCredential } from "./credential.js";
import {
  type EndpointDescriptor,
  assertPinnedLeafCertificate,
  validateEndpointDescriptor,
} from "./endpoint.js";
import {
  DeploymentMode,
  type ServerInfo,
  SystemServiceClient,
} from "./gen/colossus/api/v1alpha1/system.js";

const API_PACKAGE = "colossus.api.v1alpha1";

export interface GrpcClientConstructor<Client extends grpc.Client> {
  new (
    address: string,
    credentials: grpc.ChannelCredentials,
    options?: grpc.ClientOptions,
  ): Client;
}

function normalizePeerPin(raw: Buffer | undefined): string | undefined {
  return raw === undefined
    ? undefined
    : createHash("sha256").update(raw).digest("hex");
}

/**
 * Creates a gRPC client that trusts exactly the pinned leaf and authenticates every RPC.
 *
 * Transparent gRPC retries are disabled because an SDK cannot infer whether an
 * effectful operation is safe to replay. Durable watch reconnection is handled by the
 * explicit watch helper instead.
 */
export function assertCompatibleServerInfo(
  serverInfo: ServerInfo | undefined,
  expectedInstanceId: string,
  expectedDeploymentMode: DeploymentMode,
): void {
  if (
    expectedDeploymentMode !== DeploymentMode.DEPLOYMENT_MODE_SHARED_DAEMON &&
    expectedDeploymentMode !== DeploymentMode.DEPLOYMENT_MODE_SIDECAR
  ) {
    throw new TypeError("expected deployment mode must be shared_daemon or sidecar");
  }
  if (
    serverInfo === undefined ||
    serverInfo.instanceId !== expectedInstanceId ||
    serverInfo.deploymentMode !== expectedDeploymentMode ||
    !serverInfo.apiPackages.includes(API_PACKAGE)
  ) {
    throw new Error("authenticated Colossus server identity is incompatible");
  }
}

export async function createSecureGrpcClient<Client extends grpc.Client>(
  ClientType: GrpcClientConstructor<Client>,
  descriptor: EndpointDescriptor,
  leafCertificatePem: string | Uint8Array,
  expectedInstanceId: string,
  expectedCertificateSha256: string,
  expectedDeploymentMode: DeploymentMode,
  credential: StaticBearerCredential,
): Promise<Client> {
  const validatedDescriptor = validateEndpointDescriptor(descriptor);
  assertPinnedLeafCertificate(
    validatedDescriptor,
    leafCertificatePem,
    expectedInstanceId,
    expectedCertificateSha256,
  );
  const roots = Buffer.from(leafCertificatePem);
  const expectedHostname = validatedDescriptor.endpoint.hostname.replace(
    /^\[|\]$/gu,
    "",
  );

  const transportCredentials = grpc.credentials.createSsl(
    roots,
    undefined,
    undefined,
    {
      checkServerIdentity(_hostname, peerCertificate) {
        const peerPin = normalizePeerPin(peerCertificate.raw);
        if (peerPin !== expectedCertificateSha256) {
          return new Error("Colossus peer certificate pin mismatch");
        }
        return checkServerIdentity(expectedHostname, peerCertificate);
      },
    },
  );

  const callCredentials = grpc.credentials.createFromMetadataGenerator(
    (_parameters, callback) => {
      const metadata = new grpc.Metadata();
      credential.applyTo(metadata);
      callback(null, metadata);
    },
  );

  const channelCredentials = grpc.credentials.combineChannelCredentials(
    transportCredentials,
    callCredentials,
  );
  const options: grpc.ClientOptions = {
    "grpc.enable_http_proxy": 0,
    "grpc.enable_retries": 0,
    "grpc.max_receive_message_length": 4 * 1024 * 1024,
    "grpc.max_send_message_length": 4 * 1024 * 1024,
    "grpc.primary_user_agent": "colossus-typescript-sdk/0.10.4",
    // grpc-js always forwards its TLS servername, while Node rejects IP
    // literals in SNI. Use an inert SNI value and verify the descriptor's
    // literal IP SAN explicitly in checkServerIdentity above.
    "grpc.ssl_target_name_override": "colossus.invalid",
  };

  const client = new ClientType(
    validatedDescriptor.target,
    channelCredentials,
    options,
  );
  try {
    const system = new SystemServiceClient(
      validatedDescriptor.target,
      channelCredentials,
      {
        ...options,
        channelOverride: client.getChannel(),
      },
    );
    const serverInfo = await new Promise<ServerInfo | undefined>((resolve, reject) => {
      system.getServerInfo(
        {},
        new grpc.Metadata(),
        { deadline: Date.now() + 5_000 },
        (error, response) => {
          if (error !== null) {
            reject(error);
            return;
          }
          resolve(response.serverInfo);
        },
      );
    });
    assertCompatibleServerInfo(
      serverInfo,
      expectedInstanceId,
      expectedDeploymentMode,
    );
  } catch (error) {
    client.close();
    throw error;
  }

  return client;
}

/** Only deliberately public local failures may cross the host/webview boundary. */
export class UserError extends Error {}

export type ConnectionStage =
  | "workspace"
  | "discovery-directory"
  | "endpoint-file"
  | "endpoint-descriptor"
  | "certificate-file"
  | "certificate-pin"
  | "keyring"
  | "handshake"
  | "server-info"
  | "saved-profile"
  | "history";

export interface ConnectionDiagnostic {
  stage: ConnectionStage;
  state: "started" | "succeeded" | "failed";
  reason?: string;
}

export class ConnectionError extends UserError {
  constructor(
    readonly stage: ConnectionStage,
    readonly reason: string,
    message: string,
  ) {
    super(`Connection failed at ${stage}: ${message}`);
  }
}

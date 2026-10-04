import assert from "node:assert/strict";
import {
  chmod,
  mkdtemp,
  mkdir,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { certificateSha256 } from "@obscuritylabs/colossus-sdk";
import {
  connectWorker,
  workspaceIdentity,
  type ConnectionProfile,
} from "../src/connection.js";
import { ConnectionError, type ConnectionDiagnostic } from "../src/errors.js";

test("discovery substitution, unsafe files, and workspace replacement fail before credential access", async () => {
  const home = await mkdtemp(join(tmpdir(), "colossus-vscode-discovery-"));
  const workspace = join(home, "workspace");
  const discovery = join(home, "api");
  await mkdir(workspace);
  await mkdir(discovery, { mode: 0o700 });
  const certificate = await readFile(
    new URL("../../../../sdk/testdata/connector-cert.pem", import.meta.url),
  );
  const identity = await workspaceIdentity(workspace);
  const profile: ConnectionProfile = {
    schemaVersion: 1,
    workspacePath: identity.path,
    workspaceIdentity: identity.identity,
    discoveryDirectory: discovery,
    instanceId: "00000000-0000-4000-8000-000000000001",
    certificateSha256: certificateSha256(certificate),
    keyringService: "test.service",
    keyringAccount: "test",
    role: "primary",
  };
  const descriptor = {
    schema_version: 1,
    api_version: "colossus.api.v1alpha1",
    instance_id: profile.instanceId,
    endpoint: "https://127.0.0.1:1",
    pid: 1,
    certificate_sha256: profile.certificateSha256,
  };
  const endpoint = join(discovery, "endpoint.json");
  const pem = join(discovery, "certificate.pem");
  await writeFile(endpoint, JSON.stringify(descriptor), { mode: 0o600 });
  await writeFile(pem, certificate, { mode: 0o600 });
  let reads = 0;
  const credential = async () => {
    reads++;
    return "non-secret-fixture-token";
  };
  try {
    // A missing keychain item is distinct from malformed enrollment or TLS failure.
    const missingEvents: ConnectionDiagnostic[] = [];
    await assert.rejects(
      connectWorker(
        profile,
        async (service, account) => {
          assert.equal(service, profile.keyringService);
          assert.equal(account, profile.keyringAccount);
          return null;
        },
        (event) => missingEvents.push(event),
      ),
      (error: unknown) => {
        assert.ok(error instanceof ConnectionError);
        assert.equal(error.stage, "keyring");
        assert.equal(error.reason, "credential-not-found");
        return true;
      },
    );
    assert.deepEqual(missingEvents.at(-1), {
      stage: "keyring",
      state: "failed",
      reason: "credential-not-found",
    });
    assert.ok(!missingEvents.some((event) => event.stage === "handshake"));
    await assert.rejects(
      connectWorker(profile, async () => {
        throw new Error("sensitive-token-fixture locked-keychain");
      }),
      (error: unknown) => {
        assert.ok(error instanceof ConnectionError);
        assert.equal(error.reason, "failed");
        assert.doesNotMatch(error.message, /sensitive-token|locked-keychain/u);
        return true;
      },
    );
    const events: ConnectionDiagnostic[] = [];
    await rm(endpoint);
    await assert.rejects(
      connectWorker(profile, credential, (event) => events.push(event)),
      (error: unknown) => {
        assert.ok(error instanceof ConnectionError);
        assert.equal(error.stage, "endpoint-file");
        assert.equal(error.reason, "missing-file");
        return true;
      },
    );
    assert.deepEqual(events.at(-1), {
      stage: "endpoint-file",
      state: "failed",
      reason: "missing-file",
    });
    await writeFile(endpoint, "invalid-json", { mode: 0o600 });
    await assert.rejects(
      connectWorker(profile, credential),
      (error: unknown) => {
        assert.ok(error instanceof ConnectionError);
        assert.equal(error.stage, "endpoint-descriptor");
        return true;
      },
    );
    await writeFile(endpoint, JSON.stringify(descriptor));
    await assert.rejects(
      connectWorker(
        { ...profile, certificateSha256: "0".repeat(64) },
        credential,
      ),
      (error: unknown) => {
        assert.ok(error instanceof ConnectionError);
        assert.equal(error.stage, "certificate-pin");
        return true;
      },
    );
    await assert.rejects(
      connectWorker(
        { ...profile, workspaceIdentity: "replacement" },
        credential,
      ),
    );
    if (process.platform !== "win32") {
      await chmod(endpoint, 0o644);
      await assert.rejects(connectWorker(profile, credential));
      await chmod(endpoint, 0o600);
      await rm(pem);
      await symlink(endpoint, pem);
      await assert.rejects(connectWorker(profile, credential));
      await rm(pem);
      await writeFile(pem, certificate, { mode: 0o600 });
    }
    await writeFile(endpoint, " ".repeat(32 * 1024 + 1));
    await assert.rejects(connectWorker(profile, credential));
    assert.equal(reads, 0);
  } finally {
    await rm(home, { recursive: true, force: true });
  }
});

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import {
  assertPinnedLeafCertificate,
  certificateSha256,
  parseEndpointDescriptor,
  validateEndpointDescriptor,
} from "../src/endpoint.js";

const validDescriptor = {
  schema_version: 1,
  api_version: "colossus.api.v1alpha1",
  instance_id: "00000000-0000-4000-8000-000000000001",
  endpoint: "https://127.0.0.1:43119",
  pid: 4242,
  certificate_sha256: "a".repeat(64),
};

const missingBasicConstraintsPem = `-----BEGIN CERTIFICATE-----
MIIBUzCB+6ADAgECAhRxF4X1Ksft9+kd2EYGpZy0k26jYzAKBggqhkjOPQQDAjAZ
MRcwFQYDVQQDDA5Db2xvc3N1cy1Oby1CQzAeFw0yNjA3MTkxODU3MTNaFw0yNjA3
MjAxODU3MTNaMBkxFzAVBgNVBAMMDkNvbG9zc3VzLU5vLUJDMFkwEwYHKoZIzj0C
AQYIKoZIzj0DAQcDQgAEQgzdb3wmHXm56zDZ29gO2tU+PavNd5ABJ4OLvTZeos68
HR1eR/i20BE1HYsJixGVVMrD5YBNQaMmp0ZlDsgwGqMhMB8wHQYDVR0OBBYEFNSX
NzUVcTZbCOdXwXCVJacOy29lMAoGCCqGSM49BAMCA0cAMEQCIAtUq3vJvCtUhH7P
xBBw7d0TIX7TSOFxr/pgDzEKLZf+AiAvooM+RHDEfzLuQ8QBq7KmxWUfETCPgn69
0sDJjn619A==
-----END CERTIFICATE-----`;

test("descriptor accepts only a pinned literal loopback endpoint", () => {
  const descriptor = parseEndpointDescriptor(validDescriptor);
  assert.equal(descriptor.target, "127.0.0.1:43119");
  assert.equal(
    descriptor.instanceId,
    "00000000-0000-4000-8000-000000000001",
  );
  assert.equal(descriptor.pid, 4242);
});

test("descriptor rejects remote, plaintext, and credential-bearing endpoints", () => {
  for (const endpoint of [
    "https://example.com:43119",
    "http://127.0.0.1:43119",
    "https://user:pass@127.0.0.1:43119",
    "https://localhost:43119",
  ]) {
    assert.throws(
      () => parseEndpointDescriptor({ ...validDescriptor, endpoint }),
      /endpoint/u,
    );
  }
});

test("descriptor cannot contain a token or unknown field", () => {
  assert.throws(
    () =>
      parseEndpointDescriptor({
        ...validDescriptor,
        bearer_token: "cls_v1.should-never-be-here",
      }),
    /unsupported/u,
  );
  assert.throws(
    () =>
      parseEndpointDescriptor({
        ...validDescriptor,
        server_name: "localhost",
      }),
    /unsupported/u,
  );
});

test("descriptor JSON is bounded before parsing", () => {
  assert.throws(
    () => parseEndpointDescriptor(" ".repeat(4_097)),
    /size/u,
  );
});

test("descriptor requires exact API identity, UUID, and nonzero pid", () => {
  assert.throws(
    () => parseEndpointDescriptor({ ...validDescriptor, api_version: "v1" }),
    /api_version/u,
  );
  assert.throws(
    () => parseEndpointDescriptor({ ...validDescriptor, pid: 0 }),
    /pid/u,
  );
  assert.throws(
    () =>
      parseEndpointDescriptor({
        ...validDescriptor,
        instance_id: "not-a-uuid",
      }),
    /instance_id/u,
  );
  assert.throws(
    () =>
      parseEndpointDescriptor({
        ...validDescriptor,
        instance_id: "00000000-0000-0000-0000-000000000000",
      }),
    /instance_id/u,
  );
  assert.throws(
    () =>
      parseEndpointDescriptor({
        ...validDescriptor,
        certificate_sha256: "A".repeat(64),
      }),
    /certificate_sha256/u,
  );
});

test("descriptor rejects noncanonical loopback spellings", () => {
  for (const endpoint of [
    "https://127.1:43119",
    "https://127.0.0.1:043119",
    "https://[0:0:0:0:0:0:0:1]:43119",
  ]) {
    assert.throws(
      () => parseEndpointDescriptor({ ...validDescriptor, endpoint }),
      /endpoint/u,
    );
  }
});

test("connection-time validation rejects a forged descriptor object", () => {
  const descriptor = parseEndpointDescriptor(validDescriptor);
  assert.doesNotThrow(() => validateEndpointDescriptor(descriptor));
  assert.throws(
    () =>
      validateEndpointDescriptor({
        ...descriptor,
        target: "example.com:443",
      }),
    /inconsistent/u,
  );
  descriptor.endpoint.hostname = "example.com";
  assert.throws(() => validateEndpointDescriptor(descriptor), /endpoint/u);
});

test("public leaf must match an independent pin as well as the descriptor", () => {
  const leaf = readFileSync(
    new URL("../../../testdata/leaf.pem", import.meta.url),
    "ascii",
  );
  const pin = "a1f509c8e6096e1dbdacc7c89cb4a7895ca71d2f2c4b024449e6c2b35f8c5f0c";
  assert.equal(certificateSha256(leaf), pin);

  const descriptor = parseEndpointDescriptor({
    ...validDescriptor,
    certificate_sha256: pin,
  });
  assert.doesNotThrow(() =>
    assertPinnedLeafCertificate(
      descriptor,
      leaf,
      descriptor.instanceId,
      pin,
    ),
  );
  assert.throws(
    () =>
      assertPinnedLeafCertificate(
        parseEndpointDescriptor(validDescriptor),
        leaf,
        descriptor.instanceId,
        pin,
      ),
    /independently provisioned/u,
  );
  assert.throws(
    () =>
      assertPinnedLeafCertificate(
        descriptor,
        leaf,
        descriptor.instanceId,
        "b".repeat(64),
      ),
    /independently provisioned/u,
  );
  assert.throws(
    () =>
      assertPinnedLeafCertificate(
        descriptor,
        leaf,
        descriptor.instanceId,
        "A".repeat(64),
      ),
    /pin/u,
  );
  assert.throws(
    () =>
      assertPinnedLeafCertificate(
        descriptor,
        leaf,
        "00000000-0000-4000-8000-000000000002",
        pin,
      ),
    /instance ID/u,
  );
  assert.throws(() => certificateSha256(`${leaf}\n${leaf}`), /exactly one/u);
  assert.throws(() => certificateSha256("A".repeat(65_537)), /size/u);
  assert.throws(
    () => certificateSha256(missingBasicConstraintsPem),
    /BasicConstraints/u,
  );
});

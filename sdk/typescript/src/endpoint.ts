import { X509Certificate, createHash } from "node:crypto";

const PIN_PATTERN = /^[0-9a-f]{64}$/u;
const INSTANCE_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u;
const ENDPOINT_PATTERN =
  /^https:\/\/(?:127\.0\.0\.1|\[::1\]):([1-9][0-9]{0,4})\/?$/u;
const TARGET_PATTERN = /^(?:127\.0\.0\.1|\[::1\]):([1-9][0-9]{0,4})$/u;
const CERTIFICATE_PEM_PATTERN =
  /^\s*-----BEGIN CERTIFICATE-----\r?\n[A-Za-z0-9+/=\r\n]+-----END CERTIFICATE-----\s*$/u;
const ALLOWED_KEYS = new Set([
  "schema_version",
  "api_version",
  "instance_id",
  "endpoint",
  "pid",
  "certificate_sha256",
]);
const BASIC_CONSTRAINTS_OID = Buffer.from([0x55, 0x1d, 0x13]);

export interface EndpointDescriptor {
  readonly schemaVersion: 1;
  readonly apiVersion: "colossus.api.v1alpha1";
  readonly instanceId: string;
  readonly endpoint: URL;
  readonly target: string;
  readonly pid: number;
  readonly certificateSha256: string;
}

interface DerNode {
  readonly tag: number;
  readonly valueStart: number;
  readonly end: number;
}

function readDerNode(input: Buffer, offset: number, enclosingEnd: number): DerNode {
  if (offset < 0 || offset + 2 > enclosingEnd) {
    throw new TypeError("leaf certificate has malformed DER");
  }
  const tag = input[offset];
  const firstLength = input[offset + 1];
  if (tag === undefined || firstLength === undefined) {
    throw new TypeError("leaf certificate has malformed DER");
  }
  let length = firstLength;
  let valueStart = offset + 2;
  if ((firstLength & 0x80) !== 0) {
    const octets = firstLength & 0x7f;
    if (octets === 0 || octets > 4 || valueStart + octets > enclosingEnd) {
      throw new TypeError("leaf certificate has malformed DER");
    }
    length = 0;
    for (let index = 0; index < octets; index += 1) {
      const octet = input[valueStart + index];
      if (octet === undefined) {
        throw new TypeError("leaf certificate has malformed DER");
      }
      length = length * 256 + octet;
    }
    valueStart += octets;
  }
  const end = valueStart + length;
  if (!Number.isSafeInteger(end) || end > enclosingEnd) {
    throw new TypeError("leaf certificate has malformed DER");
  }
  return { tag, valueStart, end };
}

function derChildren(input: Buffer, parent: DerNode): DerNode[] {
  const children: DerNode[] = [];
  let offset = parent.valueStart;
  while (offset < parent.end) {
    const child = readDerNode(input, offset, parent.end);
    children.push(child);
    offset = child.end;
  }
  if (offset !== parent.end) {
    throw new TypeError("leaf certificate has malformed DER");
  }
  return children;
}

function hasBasicConstraintsExtension(raw: Buffer): boolean {
  const certificate = readDerNode(raw, 0, raw.length);
  if (certificate.tag !== 0x30 || certificate.end !== raw.length) {
    throw new TypeError("leaf certificate has malformed DER");
  }
  const certificateChildren = derChildren(raw, certificate);
  const tbs = certificateChildren[0];
  if (tbs?.tag !== 0x30) {
    throw new TypeError("leaf certificate has malformed DER");
  }
  const extensionsWrapper = derChildren(raw, tbs).find(
    (child) => child.tag === 0xa3,
  );
  if (extensionsWrapper === undefined) {
    return false;
  }
  const extensions = derChildren(raw, extensionsWrapper);
  if (extensions.length !== 1 || extensions[0]?.tag !== 0x30) {
    throw new TypeError("leaf certificate has malformed extensions");
  }
  for (const extension of derChildren(raw, extensions[0])) {
    if (extension.tag !== 0x30) {
      throw new TypeError("leaf certificate has malformed extensions");
    }
    const fields = derChildren(raw, extension);
    const oid = fields[0];
    if (
      oid?.tag === 0x06 &&
      raw.subarray(oid.valueStart, oid.end).equals(BASIC_CONSTRAINTS_OID)
    ) {
      return true;
    }
  }
  return false;
}

function requireBoundedString(
  object: Record<string, unknown>,
  field: string,
  maximum: number,
): string {
  const value = object[field];
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > maximum ||
    /[\u0000-\u001f\u007f]/u.test(value)
  ) {
    throw new TypeError(`${field} must be a non-empty bounded string`);
  }
  return value;
}

function normalizeLoopbackUrl(value: string): {
  endpoint: URL;
  target: string;
} {
  const endpointMatch = ENDPOINT_PATTERN.exec(value);
  if (endpointMatch === null) {
    throw new TypeError(
      "endpoint must be a canonical credential-free https literal-loopback URL",
    );
  }
  let endpoint: URL;
  try {
    endpoint = new URL(value);
  } catch {
    throw new TypeError("endpoint must be an absolute URL");
  }

  if (
    endpoint.protocol !== "https:" ||
    endpoint.username !== "" ||
    endpoint.password !== "" ||
    endpoint.search !== "" ||
    endpoint.hash !== "" ||
    endpoint.pathname !== "/"
  ) {
    throw new TypeError(
      "endpoint must be a credential-free https URL without path, query, or fragment",
    );
  }

  const hostname = endpoint.hostname.replace(/^\[|\]$/gu, "");
  if (hostname !== "127.0.0.1" && hostname !== "::1") {
    throw new TypeError("endpoint host must be a literal loopback address");
  }
  const port = Number(endpointMatch[1]);
  if (!Number.isInteger(port) || port < 1 || port > 65_535) {
    throw new TypeError("endpoint port is outside the valid range");
  }

  const target = hostname === "::1" ? `[::1]:${port}` : `127.0.0.1:${port}`;
  return { endpoint, target };
}

export function parseEndpointDescriptor(input: string | unknown): EndpointDescriptor {
  let decoded: unknown;
  if (typeof input === "string") {
    if (Buffer.byteLength(input, "utf8") > 4_096) {
      throw new TypeError("endpoint descriptor exceeds the size limit");
    }
    try {
      decoded = JSON.parse(input);
    } catch {
      throw new TypeError("endpoint descriptor is invalid JSON");
    }
  } else {
    decoded = input;
  }
  if (decoded === null || typeof decoded !== "object" || Array.isArray(decoded)) {
    throw new TypeError("endpoint descriptor must be a JSON object");
  }

  const object = decoded as Record<string, unknown>;
  for (const key of Object.keys(object)) {
    if (!ALLOWED_KEYS.has(key)) {
      throw new TypeError("endpoint descriptor contains an unsupported field");
    }
  }
  if (object.schema_version !== 1) {
    throw new TypeError("unsupported endpoint descriptor schema_version");
  }

  const apiVersion = requireBoundedString(object, "api_version", 64);
  if (apiVersion !== "colossus.api.v1alpha1") {
    throw new TypeError("unsupported endpoint descriptor api_version");
  }
  const instanceId = requireBoundedString(object, "instance_id", 128);
  if (
    !INSTANCE_ID_PATTERN.test(instanceId) ||
    instanceId === "00000000-0000-0000-0000-000000000000"
  ) {
    throw new TypeError("instance_id must be a canonical non-nil UUID");
  }
  const endpointValue = requireBoundedString(object, "endpoint", 256);
  const pidValue = object.pid;
  if (
    typeof pidValue !== "number" ||
    !Number.isInteger(pidValue) ||
    pidValue < 1 ||
    pidValue > 0xffff_ffff
  ) {
    throw new TypeError("pid must be a nonzero unsigned 32-bit integer");
  }
  const certificateSha256 = requireBoundedString(
    object,
    "certificate_sha256",
    64,
  );
  if (!PIN_PATTERN.test(certificateSha256)) {
    throw new TypeError("certificate_sha256 must be 64 lowercase hexadecimal digits");
  }

  const { endpoint, target } = normalizeLoopbackUrl(endpointValue);
  return {
    schemaVersion: 1,
    apiVersion,
    instanceId,
    endpoint,
    target,
    pid: pidValue,
    certificateSha256,
  };
}

export function validateEndpointDescriptor(
  descriptor: EndpointDescriptor,
): EndpointDescriptor {
  if (!(descriptor.endpoint instanceof URL)) {
    throw new TypeError("endpoint descriptor is inconsistent");
  }
  const targetMatch = TARGET_PATTERN.exec(descriptor.target);
  if (targetMatch === null) {
    throw new TypeError("endpoint descriptor is inconsistent");
  }
  const validated = parseEndpointDescriptor({
    schema_version: descriptor.schemaVersion,
    api_version: descriptor.apiVersion,
    instance_id: descriptor.instanceId,
    // URL intentionally normalizes an explicit default HTTPS port away. Rebuild
    // the already-bounded wire value from the canonical connection target so a
    // parsed :443 descriptor remains revalidatable.
    endpoint: `https://${descriptor.target}`,
    pid: descriptor.pid,
    certificate_sha256: descriptor.certificateSha256,
  });
  if (
    validated.target !== descriptor.target ||
    validated.endpoint.href !== descriptor.endpoint.href
  ) {
    throw new TypeError("endpoint descriptor is inconsistent");
  }
  return validated;
}

export function certificateSha256(leafCertificatePem: string | Uint8Array): string {
  const inputLength =
    typeof leafCertificatePem === "string"
      ? Buffer.byteLength(leafCertificatePem, "utf8")
      : leafCertificatePem.byteLength;
  if (inputLength > 65_536) {
    throw new TypeError("leaf certificate exceeds the size limit");
  }
  const pem =
    typeof leafCertificatePem === "string"
      ? leafCertificatePem
      : Buffer.from(leafCertificatePem).toString("ascii");
  if (
    !CERTIFICATE_PEM_PATTERN.test(pem) ||
    (typeof leafCertificatePem !== "string" &&
      !Buffer.from(leafCertificatePem).equals(Buffer.from(pem, "ascii")))
  ) {
    throw new TypeError("exactly one public leaf certificate PEM is required");
  }
  let certificate: X509Certificate;
  try {
    certificate = new X509Certificate(pem);
  } catch {
    throw new TypeError("leaf certificate is not valid PEM");
  }
  if (certificate.ca || !hasBasicConstraintsExtension(certificate.raw)) {
    throw new TypeError(
      "endpoint identity certificate must declare BasicConstraints CA=false",
    );
  }
  return createHash("sha256").update(certificate.raw).digest("hex");
}

export function assertPinnedLeafCertificate(
  descriptor: EndpointDescriptor,
  leafCertificatePem: string | Uint8Array,
  expectedInstanceId: string,
  expectedCertificateSha256: string,
): void {
  if (
    !INSTANCE_ID_PATTERN.test(expectedInstanceId) ||
    expectedInstanceId === "00000000-0000-0000-0000-000000000000"
  ) {
    throw new TypeError(
      "independently provisioned instance ID must be a canonical non-nil UUID",
    );
  }
  if (!PIN_PATTERN.test(expectedCertificateSha256)) {
    throw new TypeError(
      "independently provisioned certificate pin must be 64 lowercase hexadecimal digits",
    );
  }
  if (descriptor.instanceId !== expectedInstanceId) {
    throw new Error(
      "endpoint descriptor instance ID does not match the independently provisioned identity",
    );
  }
  if (descriptor.certificateSha256 !== expectedCertificateSha256) {
    throw new Error(
      "endpoint descriptor certificate pin does not match the independently provisioned pin",
    );
  }
  if (certificateSha256(leafCertificatePem) !== expectedCertificateSha256) {
    throw new Error(
      "leaf certificate does not match the independently provisioned pin",
    );
  }
}

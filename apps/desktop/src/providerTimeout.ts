export const REMOTE_PROVIDER_TIMEOUT_MS = 300_000;
export const LOOPBACK_PROVIDER_TIMEOUT_MS = 900_000;

export function automaticProviderTimeoutMs(baseUrl: string): number {
  try {
    const hostname = new URL(baseUrl).hostname.toLowerCase();
    const ipv4 = hostname.split(".").map(Number);
    const loopback =
      hostname === "localhost" ||
      hostname === "::1" ||
      hostname === "[::1]" ||
      (ipv4.length === 4 &&
        ipv4.every(
          (octet) => Number.isInteger(octet) && octet >= 0 && octet <= 255,
        ) &&
        ipv4[0] === 127);
    return loopback ? LOOPBACK_PROVIDER_TIMEOUT_MS : REMOTE_PROVIDER_TIMEOUT_MS;
  } catch {
    return REMOTE_PROVIDER_TIMEOUT_MS;
  }
}

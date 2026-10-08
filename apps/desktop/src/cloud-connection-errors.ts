import { normalizeCommandError } from "./api";

/** Show native validation details without exposing an unstructured bridge error. */
export function cloudConnectionError(value: unknown, fallback: string): string {
  const error = normalizeCommandError(value);
  const details = error.violations
    .map((violation) => violation.description.trim())
    .filter(Boolean);
  if (details.length) return details.join(" ");
  return error.code === "desktop_request_failed" ? fallback : error.message;
}

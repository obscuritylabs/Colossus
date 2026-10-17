import { isAbsolute } from "node:path";

/** Select only Cargo's fresh, runnable binaries; never fall back to an old path. */
export function nativeExecutables(messages, packages) {
  const executables = {};
  for (const message of messages) {
    if (
      message.reason !== "compiler-artifact" ||
      !packages.includes(message.target?.name) ||
      !message.target.kind?.includes("bin")
    )
      continue;
    if (
      message.profile?.test !== false ||
      typeof message.executable !== "string" ||
      !isAbsolute(message.executable)
    )
      throw new Error("Cargo did not produce a runnable native executable.");
    const name = message.target.name;
    if (executables[name] && executables[name] !== message.executable)
      throw new Error("Cargo reported ambiguous native executables.");
    executables[name] = message.executable;
  }
  if (packages.some((name) => !executables[name]))
    throw new Error("Cargo did not report every native executable.");
  return executables;
}

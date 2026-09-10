import { join, resolve } from "node:path";

// tauri-build copies staged externalBin files into its target/debug directory.
// Never let that directory overlap the freshly built runtime under test, even
// when the caller shares CARGO_TARGET_DIR across the two Cargo workspaces.
export function acceptanceTargets(repository, desktop, cargoTargetDir) {
  const runtime = resolve(repository, cargoTargetDir ?? "target");
  const native =
    cargoTargetDir === undefined
      ? join(desktop, "src-tauri/target")
      : join(runtime, "desktop-acceptance");
  return { runtime, native };
}

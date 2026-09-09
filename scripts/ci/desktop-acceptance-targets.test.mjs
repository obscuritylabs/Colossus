import assert from "node:assert/strict";
import {
  mkdtemp,
  mkdir,
  writeFile,
  copyFile,
  readFile,
  rm,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { acceptanceTargets } from "../../apps/desktop/scripts/acceptance-targets.mjs";

for (const variant of ["default", "relative", "absolute"]) {
  test(`Tauri external binaries cannot replace the acceptance runtime: ${variant}`, async () => {
    const root = await mkdtemp(join(tmpdir(), "colossus-acceptance-targets-"));
    try {
      const repository = join(root, "repository");
      const desktop = join(repository, "apps/desktop");
      const configured =
        variant === "default"
          ? undefined
          : variant === "relative"
            ? "shared"
            : join(root, "shared");
      const targets = acceptanceTargets(repository, desktop, configured);
      assert.equal(
        targets.runtime,
        resolve(repository, configured ?? "target"),
      );
      assert.notEqual(targets.runtime, targets.native);
      await mkdir(join(targets.runtime, "debug"), { recursive: true });
      await mkdir(join(targets.native, "debug"), { recursive: true });
      const staged = join(root, "staged-sidecar");
      await writeFile(staged, "old staged protocol");
      for (const name of ["colossus", "colossus-sidecar"]) {
        const current = join(targets.runtime, "debug", name);
        await writeFile(current, "current compiled protocol");
        // Reproduce tauri-build's externalBin publication after the runtime build.
        await copyFile(staged, join(targets.native, "debug", name));
        assert.equal(
          await readFile(current, "utf8"),
          "current compiled protocol",
        );
      }
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
}

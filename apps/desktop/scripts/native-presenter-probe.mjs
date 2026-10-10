import { spawnSync } from "node:child_process";
import { writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const required = [
  "native_presenter_parent_close=passed",
  "native_presenter_reentrant_close=passed",
  "native_presenter_identity_reuse=passed",
];

// This probe uses the actual platform window/focus callbacks. It does not load
// CEF, attest sandbox containment or replace the browser rendering/input tier.
export async function runNativePresenterProbe(evidenceDirectory, nativeBuild) {
  const executable = join(
    resolve(nativeBuild),
    `colossus-browser-presenter-probe${process.platform === "win32" ? ".exe" : ""}`,
  );
  const result = spawnSync(executable, [], {
    encoding: "utf8",
    timeout: 15_000,
    maxBuffer: 64 * 1024,
    windowsHide: false,
  });
  const passed =
    result.status === 0 &&
    !result.error &&
    required.every((marker) => result.stdout?.includes(marker));
  await writeFile(
    join(evidenceDirectory, "native-presenter-lifecycle.json"),
    JSON.stringify(
      {
        passed,
        platform: process.platform,
        status: result.status,
        checks: Object.fromEntries(
          required.map((marker) => [marker.split("=")[0], passed]),
        ),
      },
      null,
      2,
    ) + "\n",
    { flag: "wx", mode: 0o600 },
  );
  if (!passed) throw new Error("Native presenter lifecycle acceptance failed");
  console.log(
    "PASS native presenter parent close, reentrant close and stale identity",
  );
}

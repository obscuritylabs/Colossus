import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";

test("invalid local weights cannot become bundled model resources or leave partial files", async () => {
  const directory = await mkdtemp(join(tmpdir(), "colossus-dictation-stage-"));
  try {
    const source = join(directory, "model.bin");
    await writeFile(source, "unreviewed model bytes");
    const output = join(directory, "output");
    const result = spawnSync(
      process.execPath,
      [
        "../../scripts/stage-dictation-model.mjs",
        "--model-file",
        source,
        "--output",
        output,
      ],
      { encoding: "utf8", timeout: 10000 },
    );
    assert.equal(result.status, 1);
    assert.match(result.stderr, /checksum\/size mismatch/u);
    assert.deepEqual(await readdir(output), []);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("Desktop ships only pinned Tiny weights and retains their provenance and microphone entitlement", async () => {
  const config = JSON.parse(
    await readFile("src-tauri/tauri.conf.json", "utf8"),
  );
  const pin = JSON.parse(
    await readFile("../../release/dictation/models.json", "utf8"),
  );
  const resources = config.bundle.resources;
  assert.equal(
    resources[`dictation-assets/${pin.models.tiny_english.filename}`],
    `dictation/${pin.models.tiny_english.filename}`,
  );
  assert.equal(
    resources["dictation-assets/LICENSE-MIT"],
    "dictation/LICENSE-MIT",
  );
  assert.equal(
    resources["dictation-assets/models.json"],
    "dictation/models.json",
  );
  assert.ok(
    !Object.keys(resources).some((path) =>
      path.includes(pin.models.base_english.filename),
    ),
  );
  assert.equal(
    config.bundle.macOS.entitlements,
    "dictation.entitlements.plist",
  );
  const license = await readFile("../../release/dictation/LICENSE-MIT", "utf8");
  assert.match(license, /Copyright \(c\) 2022 OpenAI/u);
});

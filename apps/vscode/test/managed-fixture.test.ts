import assert from "node:assert/strict";
import { test } from "node:test";
import { chmod, mkdir, mkdtemp, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { homedir } from "node:os";
import { removeManagedFixture } from "./managed-fixture.js";

test("owned runtime fixtures remove immutable plugin directories and refuse unrelated homes", async () => {
  await assert.rejects(removeManagedFixture(homedir()), /unrelated fixture/u);
  const root = await mkdtemp(join(homedir(), ".colossus-vscode-managed-test-"));
  const content = join(root, "plugins", "content", "sha256", "test-plugin");
  await mkdir(content, { recursive: true });
  await writeFile(join(content, "SKILL.md"), "immutable fixture", {
    mode: 0o444,
  });
  await chmod(content, 0o555);
  await chmod(join(content, ".."), 0o555);
  await removeManagedFixture(root);
  await assert.rejects(stat(root), { code: "ENOENT" });
});

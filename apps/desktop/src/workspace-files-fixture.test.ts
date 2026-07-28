import { describe, expect, it } from "vitest";

import {
  listFixtureWorkspaceDirectory,
  readFixtureWorkspaceFile,
} from "./dev/workspace-files-fixture";

describe("workspace file fixture", () => {
  it("provides lazy directories and syntax metadata without absolute paths", async () => {
    const root = await listFixtureWorkspaceDirectory("fixture-workspace");
    const file = await readFixtureWorkspaceFile(
      "fixture-workspace",
      "apps/desktop/src/components/WorkSurface.tsx",
    );

    expect(root.entries.map((entry) => entry.name)).toContain("apps");
    expect(file.language).toBe("tsx");
    expect(file.content).toContain('activeDrawer === "files"');
    expect(JSON.stringify({ root, file })).not.toContain("/Users/");
  });
});

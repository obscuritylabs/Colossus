import { describe, expect, it } from "vitest";
import { releasedArtifacts } from "./released-artifacts";
import type { Update } from "./api";
describe("released artifact library", () => {
  it("ignores malformed released content instead of breaking the inventory", () => {
    const event = (message: unknown): Update => ({
      run_id: "run",
      sequence: 1,
      created_at: "2026-10-08T12:00:00Z",
      update: { message },
    });
    expect(
      releasedArtifacts([
        event({ content: {} }),
        event({
          content: [
            null,
            { artifact: { artifact_id: 42, file_name: "invalid" } },
          ],
        }),
        event({
          content: [
            {
              artifact: {
                artifact_id: "valid",
                file_name: "report",
                byte_length: -2,
                media_type: {},
              },
            },
          ],
        }),
      ]),
    ).toEqual([
      expect.objectContaining({
        key: "valid",
        sizeLabel: "Size not reported",
        typeLabel: "File",
      }),
    ]);
  });
  it("uses explicit released artifact metadata and deduplicates repeated messages", () => {
    const event: Update = {
      run_id: "run",
      sequence: 1,
      created_at: "2026-10-08T12:00:00Z",
      update: {
        message: {
          content: [
            { text: "Do not infer a file from prose: /private/file.txt" },
            {
              artifact: {
                artifact_id: "artifact-one",
                file_name: "report.json",
                media_type: "application/json",
                byte_length: 44,
                purpose: "output",
                state: "released",
              },
            },
          ],
        },
      },
    };
    const artifacts = releasedArtifacts([event, event]);
    expect(artifacts).toHaveLength(1);
    expect(artifacts[0]!.fileName).toBe("report.json");
    expect(artifacts[0]!.canOpen).toBe(false);
  });
});

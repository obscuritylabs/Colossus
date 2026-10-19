import { describe, expect, it } from "vitest";
import { viewedImages } from "./viewed-images";
import { presentToolActivity } from "./activity-labels";
import type { ToolActivity } from "./types";

const image = {
  artifact_id: `artifact-${"a".repeat(64)}`,
  file_name: "screen.png",
  media_type: "image/png",
  size_bytes: 100,
  sha256: "b".repeat(64),
  width_pixels: 20,
  height_pixels: 10,
  detail: "auto",
};
const activity: ToolActivity = {
  callId: "view",
  toolName: "image.view",
  state: "completed",
  summary: "Tool completed",
  preview: JSON.stringify({ images: [image] }),
};

describe("viewed image activity", () => {
  it.each([
    "browser.screenshot",
    "browser.screenshot.annotated",
    "computer.screenshot",
  ])(
    "previews the released artifact from %s with the same image validation",
    (toolName) => {
      const captured = {
        ...activity,
        toolName,
        preview: JSON.stringify({ artifact: image }),
      };
      expect(viewedImages(captured)).toEqual(viewedImages(activity));
      expect(viewedImages({ ...captured, state: "failed" })).toEqual([]);
      expect(
        viewedImages({
          ...captured,
          preview: JSON.stringify({
            artifact: {
              ...image,
              artifact_id: "https://example.test/image.png",
            },
          }),
        }),
      ).toEqual([]);
      expect(
        viewedImages({ ...captured, preview: activity.preview ?? null }),
      ).toEqual([]);
    },
  );
  it("uses completed released metadata and accurate singular and plural labels", () => {
    expect(viewedImages(activity)).toMatchObject([
      {
        artifactId: image.artifact_id,
        fileName: "screen.png",
        widthPixels: 20,
      },
    ]);
    expect(presentToolActivity(activity)).toEqual({
      kind: "image",
      title: "Viewed 1 image",
    });
    expect(
      presentToolActivity({
        ...activity,
        preview: JSON.stringify({ images: [image, image] }),
      }).title,
    ).toBe("Viewed 2 images");
  });
  it("never infers views from inputs, other tools, or failed and pending calls", () => {
    for (const value of [
      { ...activity, toolName: "mcp.call" },
      { ...activity, state: "failed" as const },
      { ...activity, state: "started" as const },
      { ...activity, preview: null, input: activity.preview ?? null },
    ])
      expect(viewedImages(value)).toEqual([]);
    expect(presentToolActivity({ ...activity, state: "failed" }).title).toBe(
      "Couldn’t view images",
    );
  });
  it("rejects malformed, remote, oversized, and unsupported image metadata", () => {
    for (const invalid of [
      { ...image, artifact_id: "https://example.test/private.png" },
      { ...image, file_name: "../private.png" },
      { ...image, media_type: "image/svg+xml" },
      { ...image, size_bytes: 16 * 1_048_576 + 1 },
      { ...image, width_pixels: 0 },
      { ...image, sha256: "bad" },
      { ...image, detail: "high" },
    ])
      expect(
        viewedImages({
          ...activity,
          preview: JSON.stringify({ images: [invalid] }),
        }),
      ).toEqual([]);
    expect(
      viewedImages({
        ...activity,
        preview: JSON.stringify({ images: Array(17).fill(image) }),
      }),
    ).toEqual([]);
    expect(viewedImages({ ...activity, preview: "{truncated" })).toEqual([]);
  });
});

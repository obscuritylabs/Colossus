import type { ToolActivity } from "./types";

export interface ViewedImage {
  artifactId: string;
  fileName: string;
  mediaType: string;
  sizeBytes: number;
  widthPixels: number;
  heightPixels: number;
}

export type ImagePreviewLoader = (
  artifactId: string,
  thumbnail: boolean,
) => Promise<string>;

/** Only successful built-in output supplies image identities; inputs and prose do not. */
export function viewedImages(activity: ToolActivity): ViewedImage[] {
  const screenshot =
    activity.toolName === "browser.screenshot" ||
    activity.toolName === "browser.screenshot.annotated";
  if (
    (activity.toolName !== "image.view" && !screenshot) ||
    activity.state !== "completed" ||
    !activity.preview ||
    activity.preview.length > 32_768
  )
    return [];
  try {
    const output: unknown = JSON.parse(activity.preview);
    if (
      !output ||
      typeof output !== "object" ||
      !(screenshot ? "artifact" in output : "images" in output)
    )
      return [];
    const released = screenshot
      ? [(output as { artifact: unknown }).artifact]
      : (output as { images: unknown }).images;
    if (!Array.isArray(released) || released.length < 1 || released.length > 16)
      return [];
    let total = 0;
    const images: ViewedImage[] = [];
    for (const image of released) {
      if (
        !image ||
        typeof image !== "object" ||
        typeof image.artifact_id !== "string" ||
        !/^artifact-[a-f0-9]{64}$/u.test(image.artifact_id) ||
        typeof image.file_name !== "string" ||
        image.file_name.length === 0 ||
        image.file_name.length > 255 ||
        /[\u0000-\u001f\u007f/\\]/u.test(image.file_name) ||
        !["image/png", "image/jpeg", "image/webp"].includes(image.media_type) ||
        !Number.isSafeInteger(image.size_bytes) ||
        image.size_bytes < 1 ||
        image.size_bytes > 16 * 1_048_576 ||
        !Number.isSafeInteger(image.width_pixels) ||
        image.width_pixels < 1 ||
        image.width_pixels > 16_384 ||
        !Number.isSafeInteger(image.height_pixels) ||
        image.height_pixels < 1 ||
        image.height_pixels > 16_384 ||
        image.width_pixels * image.height_pixels > 100_000_000 ||
        typeof image.sha256 !== "string" ||
        !/^[a-f0-9]{64}$/u.test(image.sha256) ||
        image.detail !== "auto"
      )
        return [];
      total += image.size_bytes;
      if (total > 32 * 1_048_576) return [];
      images.push({
        artifactId: image.artifact_id,
        fileName: image.file_name,
        mediaType: image.media_type,
        sizeBytes: image.size_bytes,
        widthPixels: image.width_pixels,
        heightPixels: image.height_pixels,
      });
    }
    return images;
  } catch {
    return [];
  }
}

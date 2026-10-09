import type { LibraryArtifact } from "@colossus/ui";
import type { Update } from "./api";

export function releasedArtifacts(updates: Update[]): LibraryArtifact[] {
  const items = new Map<string, LibraryArtifact>();
  for (const update of updates) {
    const message = update.update.message as { content?: unknown } | undefined;
    if (!Array.isArray(message?.content)) continue;
    for (const part of message.content) {
      if (!part || typeof part !== "object") continue;
      const artifact = part.artifact;
      if (
        !artifact ||
        typeof artifact !== "object" ||
        typeof artifact.artifact_id !== "string" ||
        !artifact.artifact_id ||
        typeof artifact.file_name !== "string" ||
        !artifact.file_name
      )
        continue;
      items.set(artifact.artifact_id, {
        key: artifact.artifact_id,
        fileName: artifact.file_name,
        typeLabel:
          typeof artifact.media_type === "string"
            ? artifact.media_type
            : "File",
        sizeLabel:
          Number.isSafeInteger(artifact.byte_length) &&
          artifact.byte_length >= 0
            ? `${artifact.byte_length.toLocaleString()} bytes`
            : "Size not reported",
        purposeLabel:
          typeof artifact.purpose === "string"
            ? artifact.purpose
            : "Released output",
        stateLabel:
          typeof artifact.state === "string"
            ? artifact.state
            : "Released metadata",
        canOpen: false,
      });
    }
  }
  return [...items.values()];
}

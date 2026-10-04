import { invoke } from "@tauri-apps/api/core";

export type DictationModelId = "tiny_english" | "base_english";
export interface DictationSettingsSnapshot {
  available: boolean;
  enabled: boolean;
  modelId: DictationModelId;
  microphoneId: string | null;
  microphones: { id: string; name: string }[];
  microphoneMissing: boolean;
  spokenPunctuation: boolean;
  active: boolean;
  downloadActive: boolean;
  models: {
    id: DictationModelId;
    name: string;
    bytes: number;
    installed: boolean;
    bundled: boolean;
  }[];
}
export type DictationSettingsDraft = Pick<
  DictationSettingsSnapshot,
  "enabled" | "modelId" | "microphoneId" | "spokenPunctuation"
>;
export const getDictationSettings = (): Promise<DictationSettingsSnapshot> =>
  invoke("get_dictation_settings");
export const saveDictationSettings = (
  request: DictationSettingsDraft,
): Promise<DictationSettingsSnapshot> =>
  invoke("save_dictation_settings", { request });
export const downloadDictationModel = (
  modelId: DictationModelId,
): Promise<DictationSettingsSnapshot> =>
  invoke("download_dictation_model", { modelId });
export const cancelDictationDownload = (): Promise<void> =>
  invoke("cancel_dictation_download");

export function dictationError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof error.message === "string"
  )
    return error.message;
  return "Dictation settings could not be updated. Try again.";
}

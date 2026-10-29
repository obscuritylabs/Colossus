import { invoke } from "@tauri-apps/api/core";
import type { ArtifactReference } from "./types";

export interface DesktopFeaturePreferences {
  schemaVersion: number;
  appshotsEnabled: boolean;
  appshotShortcut: string;
  appshotIncludeText: boolean;
  petEnabled: boolean;
  petAlwaysOnTop: boolean;
  petMotion: boolean;
  petSize: "small" | "large";
  petX: number | null;
  petY: number | null;
}
export interface DesktopFeatureContext {
  revision: number;
  preferences: DesktopFeaturePreferences;
  appshotsAvailable: boolean;
  shortcutError: string | null;
}
export interface AppshotPreview {
  captureId: string;
  windowTitle: string;
  width: number;
  height: number;
  imageDataUrl: string;
  text: string | null;
}
export type PetStatus = "idle" | "working" | "attention" | "complete";
export interface PetContext {
  status: PetStatus;
  motion: boolean;
  size: "small" | "large";
}
export function featureError(error: unknown): string {
  return typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
    ? error.message.slice(0, 512)
    : "The Desktop feature is unavailable. Try again after reopening settings.";
}
export const getDesktopFeatures = () =>
  invoke<DesktopFeatureContext>("desktop_features_context");
export const saveDesktopFeatures = (
  preferences: DesktopFeaturePreferences,
  expectedRevision: number,
) =>
  invoke<DesktopFeatureContext>("save_desktop_features", {
    preferences,
    expectedRevision,
  });
export const readAppshot = (captureId: string) =>
  invoke<AppshotPreview>("appshot_preview", { captureId });
export const discardAppshot = (captureId: string) =>
  invoke<void>("discard_appshot", { captureId });
export const attachAppshot = (
  captureId: string,
  targetId: string,
  includeText: boolean,
) =>
  invoke<ArtifactReference[]>("attach_appshot", {
    captureId,
    targetId,
    includeText,
  });
export const syncPetStatus = (status: PetStatus) =>
  invoke<void>("sync_pet_status", { status });
export const getPetContext = () => invoke<PetContext>("pet_context");
export const petAction = (action: "open" | "hide" | "save_position" | "drag") =>
  invoke<void>("pet_action", { action });

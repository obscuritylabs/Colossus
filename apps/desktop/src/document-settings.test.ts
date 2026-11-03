import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import {
  ModelSettingsEditor,
  type ModelSettingsDraft,
} from "@colossus/ui/components/ModelSettingsEditor";
import { managedModel, modelDraft } from "./components/ManagedSettingsPane";
import type { CatalogEntry, ManagedModelCatalogValue } from "./types";

const draft: ModelSettingsDraft = {
  resourceId: null,
  label: "Document model",
  profile: "documents",
  providerProfile: "remote",
  model: "synthetic-model",
  contextWindowTokens: 128_000,
  maxOutputTokens: 4_096,
  toolCalls: "auto",
  streaming: "auto",
  imageInputs: "off",
  reasoningEffort: null,
};

function editorMarkup(value: ModelSettingsDraft) {
  return renderToStaticMarkup(
    createElement(ModelSettingsEditor, {
      draft: value,
      providers: [{ profile: "remote", label: "Remote provider" }],
      busy: false,
      onChange: vi.fn(),
      onCancel: vi.fn(),
      onSave: vi.fn(),
      connectionKey: "synthetic",
      onLoadModels: vi.fn().mockResolvedValue([]),
      onSelectCatalogModel: vi.fn(),
      onProviderChange: vi.fn(),
      onModelIdentifierChange: vi.fn(),
    }),
  );
}

describe("document input settings", () => {
  it("renders legacy settings with document inputs Off independently of images", () => {
    const markup = editorMarkup({ ...draft, imageInputs: "on" });
    expect(markup).toMatch(
      /aria-label="Document inputs"[^>]*aria-valuetext="Off"/,
    );
    expect(markup).toMatch(/aria-label="Image inputs"[^>]*aria-valuetext="On"/);
    expect(markup).toContain("provider-supported PDF and Office attachments");
  });

  it("retains the explicit document preference and declaration through edit/save", () => {
    const value: ManagedModelCatalogValue = {
      profile: draft.profile,
      providerProfile: draft.providerProfile,
      model: draft.model,
      contextWindowTokens: draft.contextWindowTokens,
      maxOutputTokens: draft.maxOutputTokens,
      reasoningEffort: null,
      capabilities: {
        toolCalls: "auto",
        streaming: "auto",
        imageInputs: "off",
        documentInputs: "on",
        declared: { documentInputs: true },
      },
    };
    const entry: CatalogEntry<ManagedModelCatalogValue> = {
      id: "document-model",
      label: "Documents",
      currentRevision: 1,
      archived: false,
      revisions: [{ revision: 1, value }],
    };
    const saved = managedModel(modelDraft(entry));
    expect(saved.capabilities).toMatchObject({
      documentInputs: "on",
      declared: { documentInputs: true },
    });
    const markup = editorMarkup({
      ...draft,
      documentInputs: "auto",
      declared: { documentInputs: false },
    });
    expect(markup).toMatch(
      /aria-label="Document inputs"[^>]*aria-valuetext="Auto"/,
    );
    expect(markup).toContain("Model card reports unsupported.");
  });
});

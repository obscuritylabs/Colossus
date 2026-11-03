import assert from "node:assert/strict";
import { test } from "node:test";
import {
  ConfiguredModelProfile,
  ServerInfo,
} from "../src/gen/colossus/api/v1alpha1/system.js";

test("model document formats round trip alongside the server capability", () => {
  const profile = ConfiguredModelProfile.fromPartial({
    profile: "office",
    model: "configured-model",
    provider: "open_ai_responses",
    providerProfile: "work",
    documentMediaTypes: [
      "application/pdf",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
      "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ],
  });
  const info = ServerInfo.fromPartial({
    capabilities: [{ name: "attachments.document_input", enabled: true }],
    modelProfiles: [profile],
  });
  assert.deepEqual(ServerInfo.decode(ServerInfo.encode(info).finish()), info);
  const legacy = ConfiguredModelProfile.fromPartial({ profile: "legacy" });
  assert.deepEqual(
    ConfiguredModelProfile.decode(ConfiguredModelProfile.encode(legacy).finish())
      .documentMediaTypes,
    [],
  );
});

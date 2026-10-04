import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_PREFERENCES,
  parseSettingsAction,
  readPreferences,
} from "../src/settings.js";

test("settings changes are limited to editor preferences and fixed native actions", () => {
  for (const action of [
    { type: "setPreference", name: "sendShortcut", value: "enter" },
    { type: "setPreference", name: "defaultMode", value: "plan" },
    { type: "setPreference", name: "showToolActivity", value: false },
    { type: "setPreference", name: "palette", value: "colossus" },
    { type: "setPreference", name: "palette", value: "hacker" },
    { type: "showConnectionLog" },
    { type: "configureCredential" },
  ])
    assert.deepEqual(parseSettingsAction(action), action);
  for (const value of [
    { type: "setPreference", name: "access.danger_full_access", value: true },
    { type: "setPreference", name: "defaultMode", value: "research" },
    { type: "setPreference", name: "showToolActivity", value: "true" },
    { type: "setPreference", name: "palette", value: "arbitrary-css" },
    {
      type: "setPreference",
      name: "sendShortcut",
      value: "enter",
      target: "/other",
    },
    { type: "connect", credential: "sensitive-token-fixture" },
    { type: "configureCredential", service: "forged", account: "forged" },
    { type: "openThemeSettings", query: "secret" },
    { type: "openFile", path: "/etc/passwd" },
    { type: "setPreference", name: "__proto__", value: {} },
    {
      type: {
        toString() {
          throw new Error("untrusted");
        },
      },
    },
    null,
  ])
    assert.equal(parseSettingsAction(value), undefined);
});

test("invalid editor settings cannot change run mode or preference defaults", () => {
  assert.deepEqual(
    readPreferences(() => undefined),
    DEFAULT_PREFERENCES,
  );
  assert.deepEqual(
    readPreferences(() => "untrusted"),
    DEFAULT_PREFERENCES,
  );
  const config: Record<string, unknown> = {
    "composer.sendShortcut": "enter",
    "composer.defaultMode": "execute",
    "appearance.showToolActivity": false,
  };
  assert.deepEqual(
    readPreferences((key) => config[key]),
    {
      sendShortcut: "enter",
      defaultMode: "execute",
      showToolActivity: false,
      palette: "editor",
    },
  );
});

test("all contributed palettes round-trip through host preference validation", () => {
  for (const palette of ["editor", "colossus", "hacker"] as const) {
    const result = readPreferences((key) =>
      key === "appearance.palette" ? palette : undefined,
    );
    assert.equal(result.palette, palette);
    assert.deepEqual(
      parseSettingsAction({
        type: "setPreference",
        name: "palette",
        value: palette,
      }),
      { type: "setPreference", name: "palette", value: palette },
    );
  }
});

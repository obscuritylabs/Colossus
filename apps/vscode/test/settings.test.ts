import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import {
  DEFAULT_PREFERENCES,
  parseSettingsAction,
  readPreferences,
  preferenceSaveError,
  PREFERENCE_KEYS,
} from "../src/settings.js";

test("settings changes are limited to editor preferences and fixed native actions", () => {
  for (const action of [
    { type: "setPreference", name: "sendShortcut", value: "enter" },
    { type: "setPreference", name: "defaultMode", value: "plan" },
    { type: "setPreference", name: "defaultMode", value: "research" },
    { type: "setPreference", name: "showToolActivity", value: false },
    { type: "setPreference", name: "palette", value: "colossus" },
    { type: "setPreference", name: "palette", value: "hacker" },
    { type: "showConnectionLog" },
    { type: "configureCredential" },
    { type: "openUserSettings" },
  ])
    assert.deepEqual(parseSettingsAction(action), action);
  for (const value of [
    { type: "setPreference", name: "access.danger_full_access", value: true },
    { type: "setPreference", name: "defaultMode", value: "arbitrary" },
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
    { type: "openUserSettings", path: "/etc/passwd" },
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

test("every editable preference is registered in the extension manifest with matching defaults", () => {
  const properties = JSON.parse(readFileSync("package.json", "utf8"))
    .contributes.configuration.properties;
  for (const [name, key] of Object.entries(PREFERENCE_KEYS)) {
    const setting = properties[`colossus.${key}`];
    assert.ok(setting, `Missing contribution for ${name}`);
    assert.equal(setting.scope, "application");
    assert.equal(
      setting.default,
      DEFAULT_PREFERENCES[name as keyof typeof DEFAULT_PREFERENCES],
    );
  }
  assert.deepEqual(properties["colossus.appearance.palette"].enum, [
    "editor",
    "colossus",
    "hacker",
  ]);
  assert.deepEqual(properties["colossus.composer.defaultMode"].enum, [
    "plan",
    "execute",
    "research",
  ]);
  assert.equal(
    readPreferences((key) =>
      key === "composer.defaultMode" ? "research" : undefined,
    ).defaultMode,
    "research",
  );
});

test("preference save failures explain native settings problems without exposing private error text", () => {
  const examples = [
    [
      "Unable to write into user settings. Please open the user settings to correct errors/warnings in it and try again.",
      /contains errors/u,
    ],
    [
      "Unable to write into user settings because the file has unsaved changes.",
      /unsaved changes/u,
    ],
    [
      "Unable to write into user settings because the content of the file is newer.",
      /changed on disk/u,
    ],
    [
      "Unable to write because colossus.appearance.palette is not a registered configuration.",
      /Reload Window/u,
    ],
    [
      "Unable to write because it is configured in system policy.",
      /managed by system policy/u,
    ],
    ["EACCES: permission denied", /writable/u],
    ["unrecognized or localized error", /check the user settings file/iu],
  ] as const;
  for (const [message, category] of examples) {
    const result = preferenceSaveError(
      "palette",
      new Error(`${message} /private/user-path sensitive-token-fixture`),
    );
    assert.match(result, /Could not save Surface palette/u);
    assert.match(result, category);
    assert.doesNotMatch(
      result,
      /worker|reconnect|private\/user-path|sensitive-token/u,
    );
  }
  assert.match(
    preferenceSaveError("sendShortcut", undefined),
    /Could not save Send shortcut/u,
  );
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

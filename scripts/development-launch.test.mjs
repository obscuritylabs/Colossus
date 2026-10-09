import assert from "node:assert/strict";
import test from "node:test";
import { developmentEnvironment, selectorName } from "./development-launch.mjs";

test("development build/tool environments never contain wrapping controls or legacy journal keys", () => {
  const input = {
    PATH: "/trusted/bin",
    COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY: "/old",
    COLOSSUS_DEVELOPMENT_CREDENTIAL_KEY: "synthetic",
    COLOSSUS_DEVELOPMENT_OTHER_KEY: "synthetic",
    COLOSSUS_DEV_JOURNAL_KEY: "synthetic",
    COLOSSUS_DEV_SIGNING_KEY: "synthetic",
    COLOSSUS_JOURNAL_KEY: "synthetic",
    COLOSSUS_SIGNING_KEY: "synthetic",
  };
  assert.deepEqual(developmentEnvironment(input), { PATH: "/trusted/bin" });
  assert.deepEqual(
    developmentEnvironment(input, "/private/development-credentials"),
    {
      PATH: "/trusted/bin",
      [selectorName]: "/private/development-credentials",
    },
  );
  assert.equal(input.COLOSSUS_DEVELOPMENT_CREDENTIAL_KEY, "synthetic");
});
test("nonsecret selector rejects relative paths and control data", () => {
  assert.throws(() => developmentEnvironment({}, "relative"));
  assert.throws(() =>
    developmentEnvironment({}, "/private/authority\ninvalid"),
  );
});
test("case-insensitive Windows controls are removed while allowed spelling is preserved", () => {
  assert.deepEqual(
    developmentEnvironment({
      Path: "/trusted",
      colossus_development_credential_key: "synthetic",
      CoLoSsUs_DeVeLoPmEnT_CrEdEnTiAl_AuThOrItY: "/old",
      colossus_dev_journal_key: "synthetic",
      Colossus_Signing_Key: "synthetic",
    }),
    { Path: "/trusted" },
  );
});

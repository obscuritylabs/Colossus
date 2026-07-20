import assert from "node:assert/strict";
import { inspect } from "node:util";
import { test } from "node:test";

import { StaticBearerCredential } from "../src/credential.js";

test("credential representations are redacted", () => {
  const secret = "cls_v1.credential.very-secret-value";
  const credential = new StaticBearerCredential(secret);

  assert.equal(String(credential).includes(secret), false);
  assert.equal(inspect(credential).includes(secret), false);
  assert.equal(JSON.stringify(credential).includes(secret), false);

  const metadata = new Map<string, string>();
  credential.applyTo({
    set(name, value) {
      metadata.set(name, value);
    },
  });
  assert.equal(metadata.get("authorization"), `Bearer ${secret}`);
});

test("credential rejects whitespace and control characters", () => {
  assert.throws(
    () => new StaticBearerCredential("cls_v1.invalid token"),
    /credential/u,
  );
  assert.throws(
    () => new StaticBearerCredential("cls_v1.invalid\nheader"),
    /credential/u,
  );
  assert.throws(() => new StaticBearerCredential("x".repeat(762)), /credential/u);
});

import { expect, test } from "@playwright/test";
import { setImmediate } from "node:timers/promises";
import { AcceptanceOperations } from "./support/acceptance-operations";

test("native operation waits for matching completion, not a UI assertion deadline", async () => {
  const operations = new AcceptanceOperations();
  let clicked = false;
  let completed = false;
  const pending = operations
    .submit("package", async () => {
      clicked = true;
    })
    .then(() => {
      completed = true;
    });
  await setImmediate();
  expect(clicked).toBe(true);
  expect(completed).toBe(false);
  operations.complete("validate");
  await setImmediate();
  expect(completed).toBe(false);
  operations.complete("package");
  await pending;
  expect(completed).toBe(true);
});

test("native completion can arrive during the click and failed triggers retain their error", async () => {
  const operations = new AcceptanceOperations();
  const failure = new Error("fixture click failed");
  await expect(
    operations.submit("package", async () => {
      throw failure;
    }),
  ).rejects.toBe(failure);
  await operations.submit("package", async () => {
    operations.complete("package");
  });
});

test("missing native operation completion fails within its explicit bound", async () => {
  const operations = new AcceptanceOperations();
  await expect(operations.submit("package", async () => {}, 1)).rejects.toThrow(
    "Native package operation did not complete",
  );
});

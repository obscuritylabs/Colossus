import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import { verifyPkiReceipts } from "./browser-pki-evidence.mjs";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = resolve(desktop, "../..");
if (!["darwin", "win32"].includes(process.platform))
  throw new Error("Native Chromium PKI conformance requires macOS or Windows");
if (!process.stdin.isTTY || process.argv.length !== 2)
  throw new Error(
    "Native PKI requires an interactive operator and no arguments",
  );
if (!process.env.COLOSSUS_CEF_ROOT || !process.env.COLOSSUS_CEF_NATIVE_LIB_DIR)
  throw new Error(
    "Prepare the pinned native Chromium component before PKI conformance",
  );

// Python atomically creates an owner-private new directory, including a protected
// Windows DACL. This runner never installs, overwrites, or deletes OS-store items.
const directory = join(
  homedir(),
  `.colossus-browser-pki-fixture-${randomUUID()}`,
);
const fixture = spawn(
  process.platform === "win32" ? "python" : "python3",
  [
    "-B",
    join(repository, "native/browser/scripts/pki_fixture.py"),
    "--directory",
    directory,
  ],
  { cwd: repository, stdio: ["pipe", "pipe", "inherit"], windowsHide: true },
);
let pending = "";
let outputBytes = 0;
let fixtureFailure;
fixture.stdin.on("error", () => {
  fixtureFailure = new Error("PKI fixture control channel closed");
});
const messages = [];
const waiters = [];
const deliver = (value) => {
  const waiting = waiters.shift();
  if (waiting) waiting.resolve(value);
  else messages.push(value);
};
fixture.stdout.setEncoding("utf8");
fixture.stdout.on("data", (chunk) => {
  pending += chunk;
  outputBytes += Buffer.byteLength(chunk);
  if (outputBytes > 256 * 1024 || Buffer.byteLength(pending) > 128 * 1024) {
    fixtureFailure = new Error("PKI fixture exceeded its evidence bound");
    fixture.kill();
    return;
  }
  while (pending.includes("\n")) {
    const offset = pending.indexOf("\n");
    const line = pending.slice(0, offset);
    pending = pending.slice(offset + 1);
    try {
      deliver(JSON.parse(line));
    } catch {
      fixtureFailure = new Error("PKI fixture returned invalid evidence");
      fixture.kill();
    }
  }
});
const fixtureExited = new Promise((resolveExit) => {
  fixture.once("error", (error) => {
    fixtureFailure = error;
    for (const waiter of waiters.splice(0)) waiter.reject(error);
    resolveExit(1);
  });
  fixture.once("exit", (code) => {
    for (const waiter of waiters.splice(0))
      waiter.reject(new Error("PKI fixture exited before returning evidence"));
    resolveExit(code ?? 1);
  });
});
const nextMessage = async () => {
  if (fixtureFailure) throw fixtureFailure;
  if (fixture.exitCode !== null || fixture.killed)
    throw new Error("PKI fixture exited before returning evidence");
  if (messages.length) return messages.shift();
  return new Promise((resolveMessage, reject) => {
    const waiter = { resolve: resolveMessage, reject };
    waiters.push(waiter);
    setTimeout(() => {
      const index = waiters.indexOf(waiter);
      if (index >= 0) {
        waiters.splice(index, 1);
        reject(new Error("PKI fixture evidence timed out"));
      }
    }, 30_000).unref();
  });
};
const consoleInput = createInterface({
  input: process.stdin,
  output: process.stdout,
});
const interruption = new AbortController();
const interrupt = () => interruption.abort();
process.on("SIGINT", interrupt);
process.on("SIGTERM", interrupt);
const acknowledge = (message, required) =>
  new Promise((resolveAnswer, reject) => {
    const abort = () => {
      clearTimeout(timeout);
      reject(
        new Error(
          "Operator PKI conformance interrupted; remove only generated OS-store items",
        ),
      );
    };
    const timeout = setTimeout(() => {
      interruption.signal.removeEventListener("abort", abort);
      reject(new Error("Operator PKI step timed out"));
    }, 600_000);
    interruption.signal.addEventListener("abort", abort, { once: true });
    if (interruption.signal.aborted) return abort();
    consoleInput.question(message, (answer) => {
      clearTimeout(timeout);
      interruption.signal.removeEventListener("abort", abort);
      if (answer === required) resolveAnswer();
      else reject(new Error("Operator PKI prerequisite was not acknowledged"));
    });
  });
let native;
let metadata;
let serverReceipts;
let tlsPassed = false;
let operatorCleanupReported = false;
try {
  const ready = await nextMessage();
  if (
    ready.ready !== true ||
    ready.directory !== directory ||
    ready.fixture?.schema_version !== 1
  )
    throw new Error("PKI fixture could not establish its unique allocation");
  metadata = ready.fixture;
  console.log(`Unique operator PKI fixture: ${metadata.fixture_id}`);
  console.log(`CA DER: ${join(directory, "ca.der")}`);
  console.log(`Client PFX: ${join(directory, "client.pfx")}`);
  console.log(
    `Alternate client PFX: ${join(directory, "alternate_client.pfx")}`,
  );
  console.log(
    `Native import passphrase file (never sent to renderer/model): ${join(directory, "passphrase.txt")}`,
  );
  console.log(`CA fingerprint: ${metadata.fingerprints_sha256.ca}`);
  console.log(`Client fingerprint: ${metadata.fingerprints_sha256.client}`);
  console.log(
    `Alternate fingerprint: ${metadata.fingerprints_sha256.alternate_client}`,
  );
  console.log(
    "Use the native Browser Certificates workflow to import only these generated inputs. CA trust affects your OS user. Import both PFX identities using the private passphrase file, then quit the preview before starting the harness.",
  );
  console.log(
    "This tier proves live Chromium TLS and private-key use. Signed installed key custody/removal and full network containment remain separate acceptance requirements.",
  );
  await acknowledge(
    "Type RUN after provisioned fixture import and quitting the preview: ",
    "RUN",
  );
  native = spawn(
    process.execPath,
    [join(desktop, "scripts/test-browser-native.mjs"), "--embedded-chromium"],
    {
      cwd: repository,
      stdio: "inherit",
      env: {
        ...process.env,
        COLOSSUS_BROWSER_PKI_FIXTURE: join(directory, "fixture.json"),
      },
      windowsHide: true,
      // A terminal interrupt must not kill the owning native acceptance runner
      // before its bounded lifecycle can close and reap the actual CEF app.
      detached: process.platform !== "win32",
    },
  );
  const code = await new Promise((resolveExit, reject) => {
    native.once("error", reject);
    native.once("exit", (status) => resolveExit(status ?? 1));
  });
  fixture.stdin.write("report\n");
  const message = await nextMessage();
  serverReceipts = message.report;
  if (interruption.signal.aborted)
    throw new Error(
      "Native PKI conformance interrupted; retained fixture material requires operator cleanup",
    );
  if (code !== 0)
    throw new Error(
      "Native Chromium PKI harness failed; retained fixture files identify only test-owned items",
    );
  tlsPassed = verifyPkiReceipts(metadata, message.report);
  console.log(
    "PASS actual Chromium TLS receipts: trusted CA, denied server certificates, exact accepted/rejected client identities, and unreviewed identity redirect.",
  );
  console.log(
    "Remove only the three generated certificate fingerprints and their two generated private keys through the native OS certificate manager. Keep the private fixture material until cleanup is confirmed. Do not delete any existing user identity.",
  );
  await acknowledge(
    "Type REMOVED after removing those exact generated OS-store items: ",
    "REMOVED",
  );
  operatorCleanupReported = true;
} finally {
  consoleInput.close();
  if (native && native.exitCode === null) native.kill();
  fixture.stdin.end("stop\n");
  const timer = setTimeout(() => fixture.kill(), 5000);
  const code = await fixtureExited;
  clearTimeout(timer);
  if (metadata) {
    await writeFile(
      join(directory, "native-pki-conformance.json"),
      JSON.stringify(
        {
          schema_version: 1,
          fixture_id: metadata.fixture_id,
          platform: process.platform,
          architecture: process.arch,
          tls_conformance_passed: tlsPassed && code === 0,
          operator_cleanup_reported: operatorCleanupReported,
          os_store_cleanup_verified: false,
          signed_installed_artifact_verified: false,
          production_acceptance: false,
          fingerprints_sha256: metadata.fingerprints_sha256,
          server_receipts: serverReceipts ?? null,
          checked_at: new Date().toISOString(),
        },
        null,
        2,
      ) + "\n",
      { flag: "wx", mode: 0o600 },
    );
    console.log(`Retained private PKI material and evidence: ${directory}`);
  }
  process.removeListener("SIGINT", interrupt);
  process.removeListener("SIGTERM", interrupt);
}

import { createServer } from "node:http";
import { spawn, spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { acceptanceTargets } from "./acceptance-targets.mjs";
import { runNativePresenterProbe } from "./native-presenter-probe.mjs";
import { developmentEnvironment } from "../../../scripts/development-launch.mjs";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = resolve(desktop, "../..");
if (process.platform !== "win32" || process.arch !== "x64")
  throw new Error("Windows Chromium acceptance requires native Windows x64");
if (!process.env.COLOSSUS_CEF_ROOT || !process.env.COLOSSUS_CEF_NATIVE_LIB_DIR)
  throw new Error(
    "Run scripts/desktop-chromium-preview-windows.ps1 -BuildOnly first",
  );
const targets = acceptanceTargets(
  repository,
  desktop,
  process.env.CARGO_TARGET_DIR,
);
const run = (command, arguments_, options = {}) => {
  const result = spawnSync(command, arguments_, {
    cwd: repository,
    stdio: "inherit",
    windowsHide: true,
    ...options,
  });
  if (result.status !== 0)
    throw new Error(`Windows native acceptance command failed: ${command}`);
};
// Only fixed npm arguments use the Windows command-script shell. Paths and all
// native staging/evidence arguments use structured, direct process invocation.
run("npm.cmd", ["run", "build"], { cwd: desktop, shell: true });
run("cargo", [
  "build",
  "--locked",
  "--manifest-path",
  "apps/desktop/src-tauri/Cargo.toml",
  "--target-dir",
  targets.native,
  "--lib",
  "--features",
  "embedded-chromium-preview,browser-test-bridge,tauri/custom-protocol",
]);
const evidenceDirectory = resolve(
  process.env.COLOSSUS_BROWSER_ACCEPTANCE_EVIDENCE_DIR ??
    join(repository, ".local", `chromium-acceptance-${randomUUID()}`),
);
await mkdir(evidenceDirectory, { recursive: true });
await runNativePresenterProbe(
  evidenceDirectory,
  process.env.COLOSSUS_CEF_NATIVE_LIB_DIR,
);
const stagedApp = join(evidenceDirectory, "windows-component");
run("python", [
  "-B",
  "native/browser/scripts/stage_windows.py",
  "--cef-root",
  resolve(process.env.COLOSSUS_CEF_ROOT),
  "--native-build",
  resolve(process.env.COLOSSUS_CEF_NATIVE_LIB_DIR),
  "--client-dll",
  join(targets.native, "debug", "colossus_desktop_lib.dll"),
  "--destination",
  stagedApp,
]);
const executable = join(stagedApp, "colossus-chromium-preview.exe");
const root = join(homedir(), `.colossus-browser-acceptance-${randomUUID()}`);
const fixtureRequests = { "/first": 0, "/second": 0 };
const server = createServer((request, response) => {
  if (Object.hasOwn(fixtureRequests, request.url))
    fixtureRequests[request.url] += 1;
  response.writeHead(200, {
    "Content-Type": "text/html",
    "Cache-Control": "no-store",
  });
  response.end(
    `<!doctype html><title>${request.url === "/second" ? "Second page" : "First page"}</title><style>body{margin:0;background:white;color:black;font:20px sans-serif}h1{padding:12px}.magenta,.green{width:40vw;height:30vh;display:inline-block}.magenta{background:rgb(255,0,255)}.green{background:rgb(0,255,0)}input{position:fixed;left:12px;top:100px;width:200px;height:30px}</style><h1>Colossus native Chromium fixture</h1><a href="/second">Next page</a><div><div class="magenta"></div><div class="green"></div></div><input aria-label="Native acceptance input" oninput="if(this.value==='colossus')document.title='Native input accepted'">`,
  );
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const address = `http://127.0.0.1:${server.address().port}`;
const evidenceScript = join(
  repository,
  "native/browser/scripts/windows_process_evidence.ps1",
);
const query = (probe = false, terminate = false) => {
  const result = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      evidenceScript,
      "-Executable",
      executable,
      "-Home",
      root,
      ...(probe ? ["-Probe"] : []),
      ...(terminate ? ["-Terminate"] : []),
    ],
    {
      cwd: repository,
      encoding: "utf8",
      windowsHide: true,
      timeout: 15_000,
      maxBuffer: 256 * 1024,
    },
  );
  if (result.status !== 0)
    throw new Error("Windows owned browser process evidence failed");
  return JSON.parse(result.stdout);
};
let output = "",
  outputFailure = false,
  nativeHostPid,
  sandboxEvidence,
  cleanupEvidence;
let child, timeout;
const stop = () => {
  // Terminate only the exact acceptance host whose process handle Node retains;
  // Helpers are reaped later through separately retained, identity-checked
  // native handles. No image-name or PID-only tree kill is used.
  try {
    child?.kill();
  } catch {
    outputFailure = true;
  }
};
try {
  console.log(`Native Chromium acceptance evidence: ${evidenceDirectory}`);
  child = spawn(executable, [address], {
    cwd: repository,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: false,
    env: { ...developmentEnvironment(process.env), COLOSSUS_HOME: root },
  });
  for (const [stream, destination] of [
    [child.stdout, process.stdout],
    [child.stderr, process.stderr],
  ]) {
    stream.on("data", (chunk) => {
      destination.write(chunk);
      output += chunk.toString();
      if (Buffer.byteLength(output) > 256 * 1024) {
        outputFailure = true;
        stop();
        return;
      }
      const host = /^NATIVE_BROWSER_HOST_PID ([1-9][0-9]*)\r?\n/mu.exec(output);
      if (host) nativeHostPid = Number(host[1]);
      if (
        !sandboxEvidence &&
        output.includes(
          "PASS CEF page rendering inside Desktop Tauri/Win32 with sandbox preserved",
        )
      ) {
        try {
          sandboxEvidence = query(true);
          if (!sandboxEvidence.passed)
            throw new Error(
              "Windows renderer token/resource acceptance failed",
            );
          console.log(
            "PASS Windows renderer restricted token and OS file access denial",
          );
          output +=
            "PASS Windows renderer restricted token and OS file access denial\n";
          writeFileSync(
            join(root, "windows-sandbox-evidence.ready"),
            "accepted\n",
            { flag: "wx" },
          );
        } catch (error) {
          outputFailure = true;
          console.error(error);
          stop();
        }
      }
    });
  }
  timeout = setTimeout(() => {
    outputFailure = true;
    console.error("Windows Chromium acceptance timed out");
    stop();
  }, 240_000);
  process.exitCode = await new Promise((done) => {
    child.once("error", () => done(1));
    // close follows stdio drain; the final shutdown markers must be retained.
    child.once("close", (code) => done(code === 0 ? 0 : 1));
  });
  clearTimeout(timeout);
  const required = [
    "PASS actual Desktop controller document authorizes browser controls",
    "PASS CEF page rendering inside Desktop Tauri/Win32 with sandbox preserved",
    "PASS Windows native OS mouse and keyboard reach Chromium guest",
    "PASS Windows renderer restricted token and OS file access denial",
    "PASS CEF navigation, back, forward, and reload",
    "PASS CEF tab creation, selection, hidden sibling, and close",
    "PASS CEF native bounds and rendered content after Desktop resize",
    "PASS Desktop overlay revokes native Chromium visibility",
    "PASS stale viewport heartbeat hides CEF and fresh lease restores it",
    "PASS CEF workspace generation, foreign tab, app-origin denial, and disabled production automation",
    "PASS CEF tab close acknowledgements and temporary profile teardown",
    "PASS live CEF tab retained for native application quit",
    "PASS native application terminate requested with a live CEF tab",
    "PASS native application quit acknowledged live CEF tab close",
    "PASS native CefShutdown acknowledged after all browser close callbacks",
    "PASS CEF private application cache removed after shutdown",
    ...(process.env.COLOSSUS_BROWSER_PKI_FIXTURE
      ? ["PKI_NATIVE_TLS_CONFORMANCE_PASSED", "PKI_OS_STORE_CUSTODY_PENDING"]
      : []),
    "native browser acceptance passed",
  ];
  if (
    nativeHostPid !== child.pid ||
    outputFailure ||
    !sandboxEvidence?.passed ||
    required.some((marker) => !output.includes(marker)) ||
    fixtureRequests["/first"] < 4 ||
    fixtureRequests["/second"] < 2
  )
    process.exitCode = 1;
  const deadline = Date.now() + 5000;
  let survivors;
  do {
    survivors = query();
    if (!survivors.processQuerySucceeded || survivors.owned.length === 0) break;
    await new Promise((done) => setTimeout(done, 100));
  } while (Date.now() < deadline);
  if (!survivors.processQuerySucceeded || survivors.owned.length > 0) {
    process.exitCode = 1;
    console.error(
      "Windows Chromium helpers survived acknowledged shutdown",
      survivors.owned,
    );
  } else {
    output += "PASS no Chromium helper processes survive Desktop shutdown\n";
    console.log("PASS no Chromium helper processes survive Desktop shutdown");
  }
  if (process.exitCode !== 0) {
    try {
      cleanupEvidence = query(false, true);
      if (!cleanupEvidence.terminatedOwned)
        throw new Error("Windows browser cleanup was not acknowledged");
    } catch (error) {
      cleanupEvidence = { terminatedOwned: false };
      console.error(error.message);
    }
  }
} catch (error) {
  process.exitCode = 1;
  console.error(error.message);
  stop();
} finally {
  clearTimeout(timeout);
  if (process.exitCode !== 0 && cleanupEvidence === undefined) {
    try {
      cleanupEvidence = query(false, true);
      if (!cleanupEvidence.terminatedOwned)
        throw new Error("Windows browser cleanup was not acknowledged");
    } catch {
      cleanupEvidence = { terminatedOwned: false };
      console.error(
        "Windows browser cleanup could not be proven; private home retained",
      );
    }
  }
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
  // A failed process cleanup keeps profile evidence for operator inspection.
  // Successful acceptance alone authorizes deleting this exact generated home.
  if (process.exitCode === 0) {
    try {
      await rm(root, {
        recursive: true,
        force: true,
        maxRetries: 10,
        retryDelay: 500,
      });
    } catch {
      process.exitCode = 1;
      console.error("Windows acceptance home cleanup failed");
    }
  }
  // Keep categorical evidence even for startup errors or failed process queries.
  await writeFile(join(evidenceDirectory, "native-acceptance.log"), output);
  await writeFile(
    join(evidenceDirectory, "native-acceptance.json"),
    JSON.stringify(
      {
        passed: process.exitCode === 0,
        platform: process.platform,
        architecture: process.arch,
        stagedApp,
        executable,
        nativeHostPid: nativeHostPid ?? null,
        launchServices: false,
        chromiumSandboxRequired: true,
        sandboxEvidence: sandboxEvidence ?? null,
        cleanupEvidence: cleanupEvidence ?? null,
        productionAutomationEnabled: false,
        pkiTlsConformanceRequested: Boolean(
          process.env.COLOSSUS_BROWSER_PKI_FIXTURE,
        ),
        fixtureOrigin: address,
        fixtureRequests,
        checkedAt: new Date().toISOString(),
      },
      null,
      2,
    ) + "\n",
  );
}

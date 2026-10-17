import { createServer } from "node:http";
import { spawn, spawnSync } from "node:child_process";
import { mkdir, open as openFile, rm, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { acceptanceTargets } from "./acceptance-targets.mjs";
import { runNativePresenterProbe } from "./native-presenter-probe.mjs";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = resolve(desktop, "../..");
const chromium = process.argv.slice(2).includes("--embedded-chromium");
if (
  process.argv.slice(2).some((argument) => argument !== "--embedded-chromium")
)
  throw new Error("Usage: test-browser-native.mjs [--embedded-chromium]");
if (chromium && process.platform === "win32") {
  await import("./test-browser-chromium-windows.mjs");
  process.exit(process.exitCode ?? 0);
}
if (chromium && process.platform !== "darwin")
  throw new Error(
    "Native embedded Chromium acceptance currently requires macOS",
  );
if (
  chromium &&
  (!process.env.COLOSSUS_CEF_ROOT || !process.env.COLOSSUS_CEF_NATIVE_LIB_DIR)
)
  throw new Error(
    "Set COLOSSUS_CEF_ROOT and COLOSSUS_CEF_NATIVE_LIB_DIR to the pinned CEF source and native build",
  );
const targets = acceptanceTargets(
  repository,
  desktop,
  process.env.CARGO_TARGET_DIR,
);
if (chromium) {
  const renderer = spawnSync("npm", ["run", "build"], {
    cwd: desktop,
    stdio: "inherit",
  });
  if (renderer.status !== 0) process.exit(renderer.status ?? 1);
}
const build = spawnSync(
  "cargo",
  [
    "build",
    "--locked",
    "--manifest-path",
    "apps/desktop/src-tauri/Cargo.toml",
    "--target-dir",
    targets.native,
    "--example",
    "browser-acceptance",
    "--features",
    chromium
      ? "embedded-chromium-preview,browser-test-bridge,tauri/custom-protocol"
      : "browser-test-bridge",
  ],
  { cwd: repository, stdio: "inherit", windowsHide: true },
);
if (build.status !== 0) process.exit(build.status ?? 1);
let executable = join(
  targets.native,
  "debug/examples",
  `browser-acceptance${process.platform === "win32" ? ".exe" : ""}`,
);
let evidenceDirectory;
let stagedApp;
if (chromium) {
  evidenceDirectory = resolve(
    process.env.COLOSSUS_BROWSER_ACCEPTANCE_EVIDENCE_DIR ??
      join(repository, ".local", `chromium-acceptance-${randomUUID()}`),
  );
  await mkdir(evidenceDirectory, { recursive: true, mode: 0o700 });
  await runNativePresenterProbe(
    evidenceDirectory,
    process.env.COLOSSUS_CEF_NATIVE_LIB_DIR,
  );
  stagedApp = join(evidenceDirectory, "Colossus Chromium Acceptance.app");
  const stage = spawnSync(
    "python3",
    [
      "-B",
      join(repository, "native/browser/scripts/stage_macos.py"),
      "--cef-root",
      resolve(process.env.COLOSSUS_CEF_ROOT),
      "--native-build",
      resolve(process.env.COLOSSUS_CEF_NATIVE_LIB_DIR),
      "--executable",
      executable,
      "--app",
      stagedApp,
      "--platform",
      process.arch === "arm64" ? "macosarm64" : "macosx64",
    ],
    { cwd: repository, stdio: "inherit" },
  );
  if (stage.status !== 0) process.exit(stage.status ?? 1);
  executable = join(stagedApp, "Contents/MacOS/browser-acceptance");
  console.log(`Native Chromium acceptance evidence: ${evidenceDirectory}`);
}
// Native code creates this new directory with an owner-only DACL. Windows' shared
// temp ancestors do not satisfy Colossus home namespace-authority requirements.
const root = join(homedir(), `.colossus-browser-acceptance-${randomUUID()}`);
const downloads = {
  "/download": ["application/octet-stream", "Synthetic browser fixture"],
  "/download-html": [
    "text/html",
    "<!doctype html><title>Attachment payload</title>",
  ],
  "/download-text": ["text/plain", "Synthetic attachment text"],
  "/download-pdf": [
    "application/pdf",
    "%PDF-1.4\n% Synthetic attachment\n%%EOF",
  ],
};
const fixtureRequests = { "/first": 0, "/second": 0 };
const server = createServer((request, response) => {
  if (Object.hasOwn(fixtureRequests, request.url))
    fixtureRequests[request.url] += 1;
  const download = downloads[request.url];
  if (download) {
    response.writeHead(200, {
      "Content-Type": download[0],
      "Content-Disposition": 'attachment; filename="browser-fixture.txt"',
    });
    response.end(download[1]);
    return;
  }
  response.writeHead(200, {
    "Content-Type": "text/html",
    "Cache-Control": "no-store",
  });
  response.end(
    `<!doctype html><html><head><title>${request.url === "/second" ? "Second page" : "First page"}</title><style>body{margin:0;background:white;color:black;font:20px sans-serif}h1{padding:12px}.magenta,.green{width:40vw;height:30vh;display:inline-block}.magenta{background:rgb(255,0,255)}.green{background:rgb(0,255,0)}</style></head><body><h1>Colossus native Chromium fixture</h1><a href="/second">Next page</a><div><div class="magenta"></div><div class="green"></div></div></body></html>`,
  );
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const address = `http://127.0.0.1:${server.address().port}`;
try {
  let launchExecutable = executable;
  let launchArguments = [address];
  const logReaders = [];
  if (chromium) {
    const stdoutPath = join(evidenceDirectory, "native.stdout.log");
    const stderrPath = join(evidenceDirectory, "native.stderr.log");
    for (const [path, destination] of [
      [stdoutPath, process.stdout],
      [stderrPath, process.stderr],
    ]) {
      const file = await openFile(path, "wx", 0o600);
      await file.close();
      logReaders.push({
        file: await openFile(path, "r"),
        offset: 0,
        destination,
      });
    }
    const pause = Math.min(
      30_000,
      Math.max(
        0,
        Number.parseInt(
          process.env.COLOSSUS_BROWSER_ACCEPTANCE_PAUSE_MS ?? "0",
          10,
        ) || 0,
      ),
    );
    const foregroundWait = Math.min(
      60_000,
      Math.max(
        0,
        Number.parseInt(
          process.env.COLOSSUS_BROWSER_ACCEPTANCE_FOREGROUND_WAIT_MS ?? "15000",
          10,
        ) || 0,
      ),
    );
    launchExecutable = "/usr/bin/open";
    launchArguments = [
      "-n",
      "-W",
      "--stdout",
      stdoutPath,
      "--stderr",
      stderrPath,
      "--env",
      `COLOSSUS_HOME=${root}`,
      "--env",
      "MallocNanoZone=0",
      "--env",
      `COLOSSUS_BROWSER_ACCEPTANCE_PAUSE_MS=${pause}`,
      "--env",
      `COLOSSUS_BROWSER_ACCEPTANCE_FOREGROUND_WAIT_MS=${foregroundWait}`,
      ...(process.env.COLOSSUS_BROWSER_PKI_FIXTURE
        ? [
            "--env",
            `COLOSSUS_BROWSER_PKI_FIXTURE=${resolve(process.env.COLOSSUS_BROWSER_PKI_FIXTURE)}`,
          ]
        : []),
      stagedApp,
      "--args",
      address,
    ];
  }
  const child = spawn(launchExecutable, launchArguments, {
    cwd: repository,
    stdio: chromium ? ["ignore", "pipe", "pipe"] : "inherit",
    windowsHide: true,
    env: {
      ...process.env,
      COLOSSUS_HOME: root,
      ...(chromium ? { MallocNanoZone: "0" } : {}),
    },
  });
  let output = "";
  let nativeHostPid;
  let outputFailure = false;
  const ownedProcesses = () => {
    const result = spawnSync("/bin/ps", ["-axo", "pid=,command="], {
      encoding: "utf8",
      timeout: 3_000,
      maxBuffer: 1024 * 1024,
    });
    return {
      ...result,
      owned: (result.stdout ?? "")
        .split("\n")
        .filter((line) => line.includes(stagedApp) || line.includes(root)),
    };
  };
  let sandboxEvidence;
  const sampleSandbox = () => {
    const processes = ownedProcesses();
    const owned = processes.owned;
    const renderers = owned.filter((line) =>
      /(?:^|\s)--type=renderer(?:\s|$)/u.test(line),
    );
    const unsafeSandboxFlags =
      /(?:^|\s)--(?:no-sandbox|disable-(?:sandbox|gpu-sandbox|setuid-sandbox|seccomp-filter-sandbox|namespace-sandbox)|allow-sandbox-debugging|single-process|in-process-gpu)(?:=|\s|$)/u;
    const remoteDebugging =
      /(?:^|\s)--remote-debugging-(?:port|address|pipe)(?:=|\s|$)/u;
    sandboxEvidence = {
      ownedProcessCount: owned.length,
      rendererCount: renderers.length,
      seatbeltClientPresent:
        renderers.length > 0 &&
        renderers.every((line) =>
          /(?:^|\s)--seatbelt-client(?:=|\s|$)/u.test(line),
        ),
      unsafeSandboxFlagsAbsent: owned.every(
        (line) => !unsafeSandboxFlags.test(line),
      ),
      remoteDebuggingPortAbsent: owned.every(
        (line) => !remoteDebugging.test(line),
      ),
      processQuerySucceeded: processes.status === 0,
      checkedAt: new Date().toISOString(),
    };
    sandboxEvidence.passed =
      sandboxEvidence.processQuerySucceeded &&
      sandboxEvidence.rendererCount > 0 &&
      sandboxEvidence.seatbeltClientPresent &&
      sandboxEvidence.unsafeSandboxFlagsAbsent &&
      sandboxEvidence.remoteDebuggingPortAbsent;
    if (sandboxEvidence.passed) {
      const marker = `PASS live Chromium sandbox helpers renderer_count=${sandboxEvidence.rendererCount} seatbelt_client=true unsafe_sandbox_flags=false remote_debugging=false`;
      console.log(marker);
      output += `${marker}\n`;
    } else {
      console.error(
        "Live Chromium sandbox helper evidence failed",
        sandboxEvidence,
      );
    }
  };
  const stop = () => {
    if (chromium) {
      // LaunchServices owns the native host, while child.pid identifies only
      // open's wait proxy. Match our exact fresh app/home before signaling.
      for (const line of ownedProcesses().owned) {
        const match = /^\s*(\d+)\s+(.+)$/.exec(line);
        if (!match) continue;
        const current = spawnSync(
          "/bin/ps",
          ["-p", match[1], "-o", "command="],
          { encoding: "utf8", timeout: 3_000, maxBuffer: 1024 * 1024 },
        );
        if (current.status !== 0 || current.stdout.trim() !== match[2].trim())
          continue;
        try {
          process.kill(Number(match[1]), "SIGTERM");
        } catch (error) {
          if (error.code !== "ESRCH") console.error(error);
        }
      }
    }
    try {
      child.kill();
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
  };
  const forwardOutput = (chunk, destination) => {
    destination.write(chunk);
    output += chunk.toString();
    const host = /^NATIVE_BROWSER_HOST_PID ([1-9][0-9]*)\r?\n/mu.exec(output);
    if (!nativeHostPid && host) {
      nativeHostPid = Number(host[1]);
      console.log(`Native Chromium acceptance host PID: ${nativeHostPid}`);
    }
    if (
      !sandboxEvidence &&
      output.includes(
        "PASS CEF page rendering inside Desktop Tauri/AppKit with sandbox preserved",
      )
    )
      sampleSandbox();
    if (Buffer.byteLength(output) > 256 * 1024) {
      outputFailure = true;
      console.error("Native Chromium acceptance exceeded 256 KiB output");
      stop();
    }
  };
  const readLogs = async () => {
    for (const reader of logReaders) {
      const size = (await reader.file.stat()).size;
      if (size < reader.offset || size > 256 * 1024)
        throw new Error(
          "Native Chromium output log exceeded its bound or was truncated",
        );
      const available = size - reader.offset;
      if (available === 0) continue;
      const buffer = Buffer.alloc(available);
      const read = await reader.file.read(buffer, 0, available, reader.offset);
      reader.offset += read.bytesRead;
      forwardOutput(buffer.subarray(0, read.bytesRead), reader.destination);
    }
  };
  let polling = false;
  let pollTimer;
  if (chromium) {
    console.log(`Native Chromium LaunchServices proxy PID: ${child.pid}`);
    for (const [stream, destination] of [
      [child.stdout, process.stdout],
      [child.stderr, process.stderr],
    ]) {
      stream.on("data", (chunk) => {
        forwardOutput(chunk, destination);
      });
    }
    pollTimer = setInterval(() => {
      if (polling) return;
      polling = true;
      readLogs()
        .catch((error) => {
          outputFailure = true;
          console.error(error);
          stop();
        })
        .finally(() => {
          polling = false;
        });
    }, 100);
  }
  const timeout = setTimeout(
    () => {
      console.error("Native browser acceptance timed out");
      stop();
    },
    chromium ? 240_000 : 120_000,
  );
  process.exitCode = await new Promise((done) => {
    child.once("error", (error) => {
      console.error(error);
      done(1);
    });
    child.once("exit", (code, signal) => {
      if (code !== 0)
        console.error(
          `Native browser exited with code ${code}, signal ${signal}`,
        );
      done(code === 0 ? 0 : 1);
    });
  });
  clearTimeout(timeout);
  if (chromium) {
    clearInterval(pollTimer);
    while (polling) await new Promise((done) => setTimeout(done, 10));
    try {
      await readLogs();
    } catch (error) {
      console.error(error);
      outputFailure = true;
    } finally {
      await Promise.all(logReaders.map((reader) => reader.file.close()));
    }
    const required = [
      "PASS actual Desktop controller document authorizes browser controls",
      "PASS CEF page rendering inside Desktop Tauri/AppKit with sandbox preserved",
      "PASS live Chromium sandbox helpers",
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
      "PASS native clean shutdown after close acknowledgements",
      "PASS CEF private application cache removed after shutdown",
      "native browser acceptance passed",
      ...(process.env.COLOSSUS_BROWSER_PKI_FIXTURE
        ? ["PKI_NATIVE_TLS_CONFORMANCE_PASSED", "PKI_OS_STORE_CUSTODY_PENDING"]
        : []),
    ];
    if (
      !nativeHostPid ||
      outputFailure ||
      required.some((marker) => !output.includes(marker))
    ) {
      console.error(
        "Native Chromium exited without all required acceptance evidence",
      );
      process.exitCode = 1;
    }
    if (fixtureRequests["/first"] < 3 || fixtureRequests["/second"] < 2) {
      console.error(
        "CEF did not perform the expected native page requests",
        fixtureRequests,
      );
      process.exitCode = 1;
    } else {
      const marker = `PASS native loopback page requests first=${fixtureRequests["/first"]} second=${fixtureRequests["/second"]}`;
      console.log(marker);
      output += `${marker}\n`;
    }
    // A zero parent exit does not establish helper cleanup. Match only the
    // freshly generated app/home owned by this exact acceptance run.
    let processes;
    let survivors;
    const reapDeadline = Date.now() + 5_000;
    do {
      processes = ownedProcesses();
      survivors = processes.owned;
      if (processes.status !== 0 || survivors.length === 0) break;
      await new Promise((done) => setTimeout(done, 100));
    } while (Date.now() < reapDeadline);
    if (processes.status !== 0 || survivors.length > 0) {
      console.error("Native Chromium helpers survived shutdown", survivors);
      process.exitCode = 1;
      stop();
      for (const line of survivors) {
        const match = /^\s*(\d+)\s+(.+)$/.exec(line);
        if (!match) continue;
        const current = spawnSync(
          "/bin/ps",
          ["-p", match[1], "-o", "command="],
          { encoding: "utf8", timeout: 3_000, maxBuffer: 1024 * 1024 },
        );
        // Failed CEF children may have moved into their own process group.
        // Recheck the exact command from our fresh app/home before cleanup.
        if (current.status === 0 && current.stdout.trim() === match[2].trim()) {
          try {
            process.kill(Number(match[1]), "SIGKILL");
            output += `CLEANUP terminated failed-run Chromium helper PID ${match[1]}\n`;
          } catch (error) {
            if (error.code !== "ESRCH") console.error(error);
          }
        }
      }
    } else {
      const marker =
        "PASS no Chromium helper processes survive Desktop shutdown";
      console.log(marker);
      output += `${marker}\n`;
    }
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
          launchServices: true,
          cefRoot: resolve(process.env.COLOSSUS_CEF_ROOT),
          nativeBuild: resolve(process.env.COLOSSUS_CEF_NATIVE_LIB_DIR),
          chromiumSandboxRequired: true,
          sandboxEvidence: sandboxEvidence ?? null,
          productionAutomationEnabled: false,
          pkiTlsConformanceRequested: Boolean(
            process.env.COLOSSUS_BROWSER_PKI_FIXTURE,
          ),
          pkiOsStoreCustodyVerified: false,
          fixtureOrigin: address,
          fixtureRequests,
          checkedAt: new Date().toISOString(),
        },
        null,
        2,
      ) + "\n",
    );
  }
} finally {
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
  const cleanup = resolve(root);
  if (
    dirname(cleanup) !== resolve(homedir()) ||
    !/^\.colossus-browser-acceptance-[0-9a-f-]{36}$/.test(basename(cleanup))
  )
    throw new Error("Refusing cleanup outside the generated test home");
  await rm(cleanup, {
    recursive: true,
    force: true,
    maxRetries: 10,
    retryDelay: 500,
  });
}

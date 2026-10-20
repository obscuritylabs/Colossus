import { spawn, spawnSync } from "node:child_process";
import {
  accessSync,
  constants,
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
} from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { delimiter, dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { developmentEnvironment } from "./development-launch.mjs";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const defaultRepository = resolve(scriptDirectory, "..");
const port = 1422;
const origin = `http://127.0.0.1:${port}`;
const usage = `Usage: scripts/mobile-dev [--ios | --browser-preview] [--simulator NAME]

The default mode launches an available iPhone Simulator with native transport.
--browser-preview serves the renderer on ${origin} without native transport.
Choose Explore development preview in either mode for labeled sample data.
`;

export function parseOptions(argv) {
  const options = { mode: "ios", simulator: undefined, help: false };
  let modeSelected = false;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help" || argument === "-h") options.help = true;
    else if (argument === "--ios" || argument === "--browser-preview") {
      if (modeSelected) throw new Error(usage);
      modeSelected = true;
      options.mode = argument === "--ios" ? "ios" : "browser";
    } else if (argument === "--simulator") {
      const name = argv[++index];
      if (options.simulator !== undefined || !name || name.startsWith("--"))
        throw new Error(usage);
      options.simulator = name;
    } else throw new Error(usage);
  }
  if (options.mode === "browser" && options.simulator !== undefined)
    throw new Error("--simulator requires iOS mode\n" + usage);
  return options;
}

export function selectIphoneSimulators(value, requested) {
  if (!value?.devices || typeof value.devices !== "object")
    throw new Error("Xcode returned an invalid simulator device list");
  const phones = Object.entries(value.devices).flatMap(([runtime, devices]) =>
    /\.iOS-/u.test(runtime) && Array.isArray(devices)
      ? devices
          .filter(
            (device) =>
              device?.isAvailable === true &&
              typeof device.name === "string" &&
              (device.deviceTypeIdentifier == null
                ? /^iPhone(?:\s|$)/u.test(device.name)
                : /^com\.apple\.CoreSimulator\.SimDeviceType\.iPhone(?:-|$)/u.test(
                    device.deviceTypeIdentifier,
                  )),
          )
          .map((device) => ({ ...device, runtime }))
      : [],
  );
  const matches = requested
    ? phones.filter((device) => device.name === requested)
    : phones;
  if (matches.length === 0)
    throw new Error(
      requested
        ? `No available iPhone Simulator named ${JSON.stringify(requested)}. Available: ${phones.map((device) => device.name).join(", ") || "none"}`
        : "No available iPhone Simulator. Install an iOS runtime in Xcode Settings > Components, then create an iPhone in Device Hub or with xcrun simctl create.",
    );
  return matches.find((device) => device.state === "Booted") ?? matches[0];
}

function executable(program, environment) {
  const candidates = isAbsolute(program)
    ? [program]
    : (environment.PATH ?? "")
        .split(delimiter)
        .filter(Boolean)
        .map((directory) => join(directory, program));
  return candidates.find((candidate) => {
    try {
      accessSync(candidate, constants.X_OK);
      return true;
    } catch {
      return false;
    }
  });
}

export function ensurePortAvailable(selectedPort = port) {
  return new Promise((resolvePort, reject) => {
    const server = createServer();
    server.once("error", (error) => {
      reject(
        new Error(
          error.code === "EADDRINUSE"
            ? `127.0.0.1:${selectedPort} is already in use. Stop the existing mobile development server or other listener and rerun.`
            : `Cannot listen on 127.0.0.1:${selectedPort} (${error.code ?? "unknown error"}).`,
        ),
      );
    });
    server.listen(
      { host: "127.0.0.1", port: selectedPort, exclusive: true },
      () => server.close((error) => (error ? reject(error) : resolvePort())),
    );
  });
}

function generatedIosProject(repository) {
  const directory = join(repository, "apps/mobile/src-tauri/gen/apple");
  if (!existsSync(join(directory, "project.yml"))) return false;
  return readdirSync(directory).some(
    (entry) =>
      entry.endsWith(".xcodeproj") &&
      existsSync(join(directory, entry, "project.pbxproj")),
  );
}

function connectedDeviceReport(xcrun, environment) {
  const directory = mkdtempSync(join(tmpdir(), "colossus-mobile-devices-"));
  try {
    const output = join(directory, "devices.json");
    const result = spawnSync(
      xcrun,
      ["devicectl", "list", "devices", "--quiet", "--json-output", output],
      {
        env: environment,
        encoding: "utf8",
        timeout: 20000,
        maxBuffer: 1024 * 1024,
      },
    );
    if (result.error || result.status !== 0)
      throw new Error(
        "Cannot inspect connected Apple devices. Select full Xcode and verify xcrun devicectl list devices works before launching the simulator.",
      );
    try {
      return JSON.parse(readFileSync(output, "utf8"));
    } catch {
      throw new Error(
        "Cannot verify connected Apple devices: devicectl returned an unreadable device report.",
      );
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

export function requireSimulatorOnlyDevices(report) {
  const devices = report?.result?.devices;
  if (!Array.isArray(devices))
    throw new Error(
      "Cannot verify connected Apple devices: devicectl returned an invalid device report.",
    );
  for (const device of devices) {
    if (!device || typeof device !== "object" || Array.isArray(device))
      throw new Error(
        "Cannot verify connected Apple devices: devicectl returned an invalid device record.",
      );
    if (
      device.visibilityClass === "simulators" ||
      device.hardwareProperties?.reality === "simulated" ||
      device.properties?.hardware?.reality === "simulated"
    )
      continue;
    // Match the CLI's legacy-first availability rule. Other states, missing
    // metadata, and unpaired devices can still reach its connected-device path.
    const state =
      device.connectionProperties?.tunnelState ??
      device.properties?.connection?.state;
    if (state === "unavailable") continue;
    throw new Error(
      "A physical or unrecognized Apple device is available. Disconnect physical Apple devices before running this simulator launcher; the Tauri CLI otherwise tries connected devices first.",
    );
  }
}

export function requireEmptyIosDeployScan(result) {
  if (
    !result.error &&
    !result.signal &&
    result.status === 253 &&
    result.stdout === "" &&
    result.stderr === ""
  )
    return;
  if (!result.error && !result.signal && result.status === 0)
    throw new Error(
      "ios-deploy detected a physical Apple device. Disconnect physical Apple devices before running this simulator launcher.",
    );
  throw new Error(
    "Cannot verify connected Apple devices: ios-deploy detection failed. Verify ios-deploy --detect --json --timeout 1 works before launching the simulator.",
  );
}

function runChild(program, args, options) {
  return new Promise((resolveChild, reject) => {
    const child = spawn(program, args, { ...options, stdio: "inherit" });
    const onInterrupt = () => child.kill("SIGINT");
    const onTerminate = () => child.kill("SIGTERM");
    process.on("SIGINT", onInterrupt);
    process.on("SIGTERM", onTerminate);
    const cleanup = () => {
      process.off("SIGINT", onInterrupt);
      process.off("SIGTERM", onTerminate);
    };
    child.once("error", (error) => {
      cleanup();
      reject(error);
    });
    child.once("exit", (code, signal) => {
      cleanup();
      resolveChild(code ?? (signal === "SIGINT" ? 130 : 1));
    });
  });
}

export async function launchMobile(argv, overrides = {}) {
  const runtime = {
    repository: defaultRepository,
    platform: process.platform,
    arch: process.arch,
    nodeVersion: process.versions.node,
    node: process.execPath,
    environment: process.env,
    executable,
    runSync: spawnSync,
    run: runChild,
    ensurePortAvailable,
    generatedIosProject,
    connectedDeviceReport,
    readFile: (path) => readFileSync(path, "utf8"),
    log: console.log,
    ...overrides,
  };
  const options = parseOptions(argv);
  if (options.help) {
    runtime.log(usage);
    return 0;
  }
  const [major, minor] = runtime.nodeVersion.split(".").map(Number);
  if (major < 22 || (major === 22 && minor < 12))
    throw new Error("Node.js 22.12 or newer is required");
  const environment = developmentEnvironment(runtime.environment);
  const requireExecutable = (program, advice) => {
    const path = runtime.executable(program, environment);
    if (!path) throw new Error(`${program} is required. ${advice}`);
    return path;
  };
  const npm = requireExecutable("npm", "Install Node.js with npm.");
  const capture = (program, args, advice) => {
    const result = runtime.runSync(program, args, {
      env: environment,
      encoding: "utf8",
      timeout: 20000,
      maxBuffer: 1024 * 1024,
    });
    if (result.error || result.status !== 0) throw new Error(advice);
    return result.stdout.trim();
  };
  let simulator;
  let checkConnectedDevices;
  if (options.mode === "ios") {
    if (runtime.platform !== "darwin")
      throw new Error(
        "iOS development requires macOS. Use --browser-preview here.",
      );
    const rustup = requireExecutable("rustup", "Install Rust using rustup.");
    const pin = runtime
      .readFile(join(runtime.repository, "rust-toolchain.toml"))
      .match(/^\s*channel\s*=\s*"(\d+\.\d+\.\d+)"\s*(?:#.*)?$/mu)?.[1];
    if (!pin)
      throw new Error(
        "Cannot read the pinned Rust channel in rust-toolchain.toml",
      );
    for (const [variable, tool] of [
      ["CARGO", "cargo"],
      ["RUSTC", "rustc"],
      ["RUSTDOC", "rustdoc"],
    ]) {
      const path = capture(
        rustup,
        ["which", "--toolchain", pin, tool],
        `Rust ${pin} is required. Run rustup toolchain install ${pin}.`,
      );
      if (!isAbsolute(path) || !runtime.executable(path, environment))
        throw new Error(`The pinned Rust ${tool} executable is unavailable`);
      environment[variable] = path;
    }
    environment.PATH = `${dirname(environment.CARGO)}${delimiter}${environment.PATH ?? ""}`;
    environment.RUSTUP_TOOLCHAIN = pin;
    const target =
      runtime.arch === "arm64"
        ? "aarch64-apple-ios-sim"
        : runtime.arch === "x64"
          ? "x86_64-apple-ios"
          : undefined;
    if (!target)
      throw new Error(
        `Unsupported iOS Simulator host architecture: ${runtime.arch}`,
      );
    const targets = capture(
      rustup,
      ["target", "list", "--toolchain", pin, "--installed"],
      `Cannot inspect Rust ${pin} targets. Run rustup target add --toolchain ${pin} ${target}.`,
    );
    if (!targets.split(/\s+/u).includes(target))
      throw new Error(
        `Install the simulator target: rustup target add --toolchain ${pin} ${target}`,
      );
    const xcrun = requireExecutable(
      "xcrun",
      "Install full Xcode and select it with xcode-select.",
    );
    const xcodebuild = requireExecutable(
      "xcodebuild",
      "Install full Xcode and launch it once.",
    );
    requireExecutable("xcodegen", "Run brew install xcodegen.");
    requireExecutable("pod", "Run brew install cocoapods.");
    const iosDeploy = requireExecutable(
      "ios-deploy",
      "Run brew install ios-deploy.",
    );
    capture(
      xcodebuild,
      ["-checkFirstLaunchStatus"],
      "Launch Xcode and finish its first-launch setup and license acceptance.",
    );
    capture(
      xcrun,
      ["--sdk", "iphonesimulator", "--show-sdk-path"],
      "The iPhone Simulator SDK is unavailable. Install and select full Xcode.",
    );
    let devices;
    try {
      devices = JSON.parse(
        capture(
          xcrun,
          ["simctl", "list", "devices", "available", "--json"],
          "Cannot inspect iOS Simulators. Launch Xcode and Simulator, then retry.",
        ),
      );
    } catch (error) {
      if (error instanceof SyntaxError)
        throw new Error("Xcode returned an invalid simulator device list");
      throw error;
    }
    simulator = selectIphoneSimulators(devices, options.simulator);
    checkConnectedDevices = () => {
      requireSimulatorOnlyDevices(
        runtime.connectedDeviceReport(xcrun, environment),
      );
      // Tauri also falls back to ios-deploy when devicectl has no physical devices.
      // ios-deploy 1.12.2 reports a completed, empty detection with exit code 253.
      requireEmptyIosDeployScan(
        runtime.runSync(iosDeploy, ["--detect", "--json", "--timeout", "1"], {
          env: environment,
          encoding: "utf8",
          timeout: 10000,
          maxBuffer: 1024 * 1024,
        }),
      );
    };
    checkConnectedDevices();
    runtime.log(`Using Rust ${pin} and iPhone Simulator ${simulator.name}.`);
  }
  await runtime.ensurePortAvailable();
  const mobile = join(runtime.repository, "apps/mobile");
  const launch = async (args) => {
    const status = await runtime.run(
      runtime.node,
      [
        join(runtime.repository, "scripts/development-launch.mjs"),
        "--",
        npm,
        ...args,
      ],
      { cwd: mobile, env: environment },
    );
    if (status !== 0) {
      const error = new Error(
        `Mobile development command exited with status ${status}`,
      );
      error.exitCode = status;
      throw error;
    }
  };
  await launch([
    "ci",
    "--ignore-scripts",
    "--legacy-peer-deps",
    "--install-links",
  ]);
  if (options.mode === "browser") {
    runtime.log(
      `Open ${origin} and choose Explore development preview for sample data.`,
    );
    await launch([
      "run",
      "dev",
      "--",
      "--host",
      "127.0.0.1",
      "--port",
      String(port),
      "--strictPort",
    ]);
  } else {
    if (!runtime.generatedIosProject(runtime.repository))
      await launch(["run", "ios:init", "--", "--ci", "--skip-targets-install"]);
    checkConnectedDevices();
    runtime.log(
      "Choose Explore development preview for sample data, or connect to your Control Plane. Press Ctrl+C here to stop development.",
    );
    await launch([
      "run",
      "ios:dev",
      "--",
      "--config",
      JSON.stringify({
        build: {
          beforeDevCommand: "npm run dev -- --host 127.0.0.1",
          devUrl: origin,
        },
      }),
      simulator.name,
      "--",
      "--locked",
    ]);
  }
  return 0;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    process.exitCode = await launchMobile(process.argv.slice(2));
  } catch (error) {
    console.error(`mobile-dev: ${error.message}`);
    process.exitCode = error.exitCode ?? 1;
  }
}

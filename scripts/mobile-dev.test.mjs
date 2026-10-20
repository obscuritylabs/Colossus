import assert from "node:assert/strict";
import { createServer } from "node:net";
import test from "node:test";
import {
  ensurePortAvailable,
  launchMobile,
  parseOptions,
} from "./mobile-dev.mjs";

function fixture(changes = {}) {
  const commands = [];
  const probes = [];
  const devices = {
    devices: {
      "com.apple.CoreSimulator.SimRuntime.iOS-26-0": [
        { name: "iPhone 17", isAvailable: false, state: "Shutdown" },
        { name: "iPad Pro", isAvailable: true, state: "Booted" },
        { name: "iPhone 17 Pro", isAvailable: true, state: "Shutdown" },
        { name: "iPhone 17 Pro Max", isAvailable: true, state: "Booted" },
      ],
      "com.apple.CoreSimulator.SimRuntime.watchOS-26-0": [
        { name: "iPhone unrelated", isAvailable: true, state: "Booted" },
      ],
    },
  };
  const runtime = {
    repository: "/fixture/Colossus with spaces",
    platform: "darwin",
    arch: "arm64",
    nodeVersion: "24.0.0",
    node: "/fixture/node",
    environment: {
      PATH: "/opt/homebrew/bin:/usr/bin",
      CARGO: "/opt/homebrew/bin/cargo",
      RUSTC: "/opt/homebrew/bin/rustc",
      RUSTDOC: "/opt/homebrew/bin/rustdoc",
      RUSTUP_TOOLCHAIN: "old-override",
      COLOSSUS_DEVELOPMENT_CREDENTIAL_KEY: "synthetic",
      COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY: "/unrelated-authority",
      COLOSSUS_JOURNAL_KEY: "synthetic",
    },
    executable: (program) =>
      program.startsWith("/") ? program : `/fixture/tools/${program}`,
    readFile: () => '[toolchain]\nchannel = "1.96.0"\n',
    runSync: (program, args, options) => {
      probes.push({ program, args, env: { ...options.env } });
      if (args[0] === "--detect")
        return { status: 253, stdout: "", stderr: "" };
      let stdout = "";
      if (args[0] === "which") stdout = `/fixture/rust/1.96.0/bin/${args[3]}`;
      if (args[0] === "target")
        stdout = "aarch64-apple-ios-sim\nx86_64-apple-ios\n";
      if (args[0] === "--sdk") stdout = "/fixture/iPhoneSimulator.sdk";
      if (args[0] === "simctl") stdout = JSON.stringify(devices);
      return { status: 0, stdout };
    },
    run: async (program, args, options) => {
      commands.push({
        program,
        args,
        cwd: options.cwd,
        env: { ...options.env },
      });
      return 0;
    },
    ensurePortAvailable: async () => {},
    generatedIosProject: () => false,
    connectedDeviceReport: () => ({ result: { devices: [] } }),
    log: () => {},
    ...changes,
  };
  return { runtime, commands, probes, devices };
}

function npmArguments(command) {
  assert.equal(command.program, "/fixture/node");
  assert.equal(
    command.args[0],
    "/fixture/Colossus with spaces/scripts/development-launch.mjs",
  );
  assert.equal(command.args[1], "--");
  assert.equal(command.args[2], "/fixture/tools/npm");
  assert.equal(command.cwd, "/fixture/Colossus with spaces/apps/mobile");
  return command.args.slice(3);
}

test("browser preview works without native tooling and keeps the server on loopback", async () => {
  const { runtime, commands } = fixture({
    platform: "linux",
    executable: (program) =>
      program === "npm" ? "/fixture/tools/npm" : undefined,
    runSync: () =>
      assert.fail("Browser preview must not inspect Rust or Xcode"),
  });
  assert.equal(await launchMobile(["--browser-preview"], runtime), 0);
  assert.deepEqual(npmArguments(commands[0]), [
    "ci",
    "--ignore-scripts",
    "--legacy-peer-deps",
    "--install-links",
  ]);
  assert.deepEqual(npmArguments(commands[1]), [
    "run",
    "dev",
    "--",
    "--host",
    "127.0.0.1",
    "--port",
    "1422",
    "--strictPort",
  ]);
  for (const command of commands) {
    assert.equal(command.env.COLOSSUS_DEVELOPMENT_CREDENTIAL_KEY, undefined);
    assert.equal(
      command.env.COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY,
      undefined,
    );
    assert.equal(command.env.COLOSSUS_JOURNAL_KEY, undefined);
  }
});

test("iOS launches an available iPhone with pinned Rust and initializes only generated files", async () => {
  const { runtime, commands, probes } = fixture();
  await launchMobile([], runtime);
  assert.equal(commands.length, 3);
  for (const command of commands) {
    assert.equal(command.env.CARGO, "/fixture/rust/1.96.0/bin/cargo");
    assert.equal(command.env.RUSTC, "/fixture/rust/1.96.0/bin/rustc");
    assert.equal(command.env.RUSTDOC, "/fixture/rust/1.96.0/bin/rustdoc");
    assert.equal(command.env.RUSTUP_TOOLCHAIN, "1.96.0");
    assert.equal(command.env.PATH.split(":")[0], "/fixture/rust/1.96.0/bin");
  }
  assert.ok(
    probes.some((probe) => probe.args.join(" ") === "-checkFirstLaunchStatus"),
  );
  assert.ok(
    probes.some(
      (probe) =>
        probe.args.join(" ") === "--sdk iphonesimulator --show-sdk-path",
    ),
  );
  assert.deepEqual(npmArguments(commands[1]), [
    "run",
    "ios:init",
    "--",
    "--ci",
    "--skip-targets-install",
  ]);
  const args = npmArguments(commands[2]);
  assert.deepEqual(args.slice(0, 4), ["run", "ios:dev", "--", "--config"]);
  assert.deepEqual(JSON.parse(args[4]), {
    build: {
      beforeDevCommand: "npm run dev -- --host 127.0.0.1",
      devUrl: "http://127.0.0.1:1422",
    },
  });
  assert.deepEqual(args.slice(5), ["iPhone 17 Pro Max", "--", "--locked"]);
});

test("explicit simulator selection preserves its full name and reuses an existing project", async () => {
  const { runtime, commands } = fixture({
    generatedIosProject: () => true,
    arch: "x64",
  });
  await launchMobile(["--ios", "--simulator", "iPhone 17 Pro"], runtime);
  assert.equal(commands.length, 2);
  assert.deepEqual(npmArguments(commands[1]).slice(-3), [
    "iPhone 17 Pro",
    "--",
    "--locked",
  ]);
});

test("a renamed iPhone is selected by its device type", async () => {
  const { runtime, commands, devices } = fixture();
  devices.devices = {
    "com.apple.CoreSimulator.SimRuntime.iOS-27-1": [
      {
        name: "Colossus iPhone",
        deviceTypeIdentifier:
          "com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro",
        isAvailable: true,
        state: "Shutdown",
      },
    ],
  };
  await launchMobile(["--simulator", "Colossus iPhone"], runtime);
  assert.deepEqual(npmArguments(commands.at(-1)).slice(-3), [
    "Colossus iPhone",
    "--",
    "--locked",
  ]);
});

test("an iPad named iPhone cannot be selected as an iPhone simulator", async () => {
  const { runtime, commands, devices } = fixture();
  devices.devices = {
    "com.apple.CoreSimulator.SimRuntime.iOS-27-1": [
      {
        name: "iPhone 18 Pro",
        deviceTypeIdentifier:
          "com.apple.CoreSimulator.SimDeviceType.iPad-Pro-13-inch-M5-16GB",
        isAvailable: true,
        state: "Booted",
      },
    ],
  };
  await assert.rejects(
    launchMobile([], runtime),
    /No available iPhone Simulator/u,
  );
  assert.equal(commands.length, 0);
});

test("simulated device records and unavailable physical devices permit simulator development", async () => {
  const { runtime, commands } = fixture({
    connectedDeviceReport: () => ({
      result: {
        devices: [
          { visibilityClass: "simulators" },
          { hardwareProperties: { reality: "simulated" } },
          { properties: { hardware: { reality: "simulated" } } },
          { connectionProperties: { tunnelState: "unavailable" } },
          { properties: { connection: { state: "unavailable" } } },
        ],
      },
    }),
  });
  await launchMobile([], runtime);
  assert.equal(commands.length, 3);
});

test("physical or unrecognized devices fail before installation, including unpaired devices", async () => {
  for (const device of [
    {
      hardwareProperties: { reality: "physical" },
      connectionProperties: {
        tunnelState: "connected",
        pairingState: "unpaired",
      },
    },
    {
      properties: {
        hardware: { reality: "physical" },
        connection: { state: "connected" },
      },
    },
    {
      connectionProperties: { tunnelState: "connected" },
      properties: { connection: { state: "unavailable" } },
    },
    {},
  ]) {
    const { runtime, commands } = fixture({
      connectedDeviceReport: () => ({ result: { devices: [device] } }),
    });
    await assert.rejects(
      launchMobile([], runtime),
      /Disconnect physical Apple devices/u,
    );
    assert.equal(commands.length, 0);
  }
});

test("unreadable or failed physical-device inspection stops before npm mutations", async () => {
  for (const report of [
    undefined,
    {},
    { result: { devices: null } },
    { result: { devices: [null] } },
  ]) {
    const { runtime, commands } = fixture({
      connectedDeviceReport: () => report,
    });
    await assert.rejects(
      launchMobile([], runtime),
      /Cannot verify connected Apple devices/u,
    );
    assert.equal(commands.length, 0);
  }
  const { runtime, commands } = fixture({
    connectedDeviceReport: () => {
      throw new Error("Cannot inspect connected Apple devices");
    },
  });
  await assert.rejects(
    launchMobile([], runtime),
    /Cannot inspect connected Apple devices/u,
  );
  assert.equal(commands.length, 0);
});

test("ios-deploy physical detections and failed or malformed empty scans stop before npm", async () => {
  for (const result of [
    { status: 0, stdout: '{"Event":"DeviceDetected"}', stderr: "" },
    { status: 253, stdout: "unexpected output", stderr: "" },
    { status: 253, stdout: "", stderr: "device service failed" },
    { status: 1, stdout: "", stderr: "" },
    { status: null, signal: "SIGTERM", stdout: "", stderr: "" },
  ]) {
    const { runtime, commands } = fixture();
    const probe = runtime.runSync;
    runtime.runSync = (program, args, options) =>
      args[0] === "--detect" ? result : probe(program, args, options);
    await assert.rejects(
      launchMobile([], runtime),
      /physical Apple device|ios-deploy detection failed/u,
    );
    assert.equal(commands.length, 0);
  }
});

test("a device connected during setup is caught before starting iOS development", async () => {
  const { runtime, commands } = fixture();
  let inspections = 0;
  runtime.connectedDeviceReport = () => ({
    result: {
      devices:
        inspections++ === 0
          ? []
          : [{ hardwareProperties: { reality: "physical" } }],
    },
  });
  await assert.rejects(
    launchMobile([], runtime),
    /Disconnect physical Apple devices/u,
  );
  assert.equal(commands.length, 2);
  assert.deepEqual(npmArguments(commands[1]).slice(0, 2), ["run", "ios:init"]);
});

test("missing SDK tooling, simulator targets, or runtime stop before npm can mutate dependencies", async () => {
  for (const missing of ["xcodegen", "pod", "ios-deploy"]) {
    const { runtime, commands } = fixture({
      executable: (program) =>
        program === missing
          ? undefined
          : program.startsWith("/")
            ? program
            : `/fixture/tools/${program}`,
    });
    await assert.rejects(
      launchMobile([], runtime),
      new RegExp(`${missing} is required`, "u"),
    );
    assert.equal(commands.length, 0);
  }
  const absentTarget = fixture();
  const originalProbe = absentTarget.runtime.runSync;
  absentTarget.runtime.runSync = (program, args, options) =>
    args[0] === "target"
      ? { status: 0, stdout: "aarch64-apple-darwin" }
      : originalProbe(program, args, options);
  await assert.rejects(
    launchMobile([], absentTarget.runtime),
    /rustup target add --toolchain 1\.96\.0 aarch64-apple-ios-sim/u,
  );
  assert.equal(absentTarget.commands.length, 0);
  const absentRuntime = fixture();
  absentRuntime.devices.devices = {};
  await assert.rejects(
    launchMobile([], absentRuntime.runtime),
    /No available iPhone Simulator/u,
  );
  assert.equal(absentRuntime.commands.length, 0);
});

test("unknown or unavailable simulator names fail before installation", async () => {
  for (const name of ["iPhone 17", "iPhone", "iPad Pro"]) {
    const { runtime, commands } = fixture();
    await assert.rejects(
      launchMobile(["--simulator", name], runtime),
      /No available iPhone Simulator named/u,
    );
    assert.equal(commands.length, 0);
  }
});

test("occupied port errors preserve existing listeners and stop before npm installation", async () => {
  const listener = createServer();
  await new Promise((resolve, reject) => {
    listener.once("error", reject);
    listener.listen(0, "127.0.0.1", resolve);
  });
  try {
    const selectedPort = listener.address().port;
    const { runtime, commands } = fixture({
      ensurePortAvailable: () => ensurePortAvailable(selectedPort),
    });
    await assert.rejects(
      launchMobile(["--browser-preview"], runtime),
      /already in use/u,
    );
    assert.equal(commands.length, 0);
    assert.equal(listener.listening, true);
  } finally {
    await new Promise((resolve) => listener.close(resolve));
  }
});

test("unsupported hosts, old Node, and conflicting options fail without launching subprocesses", async () => {
  for (const changes of [{ platform: "linux" }, { nodeVersion: "22.11.0" }]) {
    const { runtime, commands } = fixture(changes);
    await assert.rejects(launchMobile([], runtime));
    assert.equal(commands.length, 0);
  }
  for (const args of [
    ["--browser-preview", "--ios"],
    ["--simulator"],
    ["--browser-preview", "--simulator", "iPhone 17 Pro"],
    ["--unexpected"],
  ])
    assert.throws(() => parseOptions(args));
});

test("a failed initialization preserves its exit code and never starts the app", async () => {
  const { runtime, commands } = fixture();
  const successfulRun = runtime.run;
  runtime.run = async (...args) => {
    await successfulRun(...args);
    return commands.length === 2 ? 7 : 0;
  };
  await assert.rejects(
    launchMobile([], runtime),
    (error) => error.exitCode === 7,
  );
  assert.equal(commands.length, 2);
});

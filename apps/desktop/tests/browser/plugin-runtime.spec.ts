import { expect, test } from "@playwright/test";
import type { ChildProcess } from "node:child_process";
import {
  chmod,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { AcceptanceProcesses } from "./support/acceptance-processes";
import { AcceptanceOperations } from "./support/acceptance-operations";

// Explicit, separate acceptance tier: no test server or runtime implementation is
// linked into production Desktop. The driver compiles the production native adapter.
test.skip(
  process.env.COLOSSUS_PLUGIN_RUNTIME_ACCEPTANCE !== "1",
  "Run npm run test:plugin-runtime after building the acceptance binaries",
);

async function removePrivateFixture(path: string) {
  // Only the fresh mkdtemp tree created by this test; never follow links. Content
  // directories are intentionally read-only while installed, so unlock after exit.
  await chmod(path, 0o700);
  for (const entry of await readdir(path, { withFileTypes: true })) {
    if (entry.isDirectory() && !entry.isSymbolicLink())
      await removePrivateFixture(join(path, entry.name));
  }
  await rm(path, {
    recursive: true,
    force: true,
    maxRetries: 5,
    retryDelay: 100,
  });
}

test("browser → production native adapter → authenticated worker: offline core, previews, approval, lifecycle and authoring", async ({
  page,
}) => {
  test.setTimeout(180_000);
  const repository = resolve("../..");
  const suffix = process.platform === "win32" ? ".exe" : "";
  const bridge =
    process.env.COLOSSUS_PLUGIN_TEST_BRIDGE ??
    resolve(`src-tauri/target/debug/examples/plugin-test-bridge${suffix}`);
  const temporary = await realpath(
    await mkdtemp(
      join(process.platform === "win32" ? homedir() : tmpdir(), "cp-"),
    ),
  );
  await chmod(temporary, 0o700);
  const workspace = join(temporary, "work");
  await mkdir(workspace);
  const binary = join(temporary, `colossus${suffix}`);
  await cp(
    process.env.COLOSSUS_PLUGIN_TEST_CLI ??
      join(repository, `target/debug/colossus${suffix}`),
    binary,
  );
  const env = {
    ...process.env,
    HOME: temporary,
    COLOSSUS_HOME: join(temporary, "home"),
    COLOSSUS_RELEASE_JOURNAL_KEY: "5".repeat(64),
    COLOSSUS_RELEASE_SIGNING_KEY: "6".repeat(64),
    HTTP_PROXY: "http://127.0.0.1:1",
    HTTPS_PROXY: "http://127.0.0.1:1",
  };
  const config = join(workspace, "config.yaml");
  const smoke = await readFile(
    join(repository, "release/smoke-config.yaml"),
    "utf8",
  );
  await writeFile(
    config,
    smoke
      .replace(
        "allow: [plugin.list]",
        "allow: [plugin.list, plugin.inspect, plugin.skill.read, plugin.resource.list, plugin.resource.read, plugin.validate, plugin.verify, plugin.install, plugin.enable, plugin.disable, plugin.gc, plugin.package, plugin.export, plugin.uninstall]",
      )
      .replace(
        "filesystem: []",
        `filesystem: [{ root: ${JSON.stringify(workspace)}, mode: write }]`,
      ) + "\nplugins:\n  trustProfiles:\n    offline:\n      mode: optional\n",
  );
  const source = join(workspace, "example");
  await mkdir(join(source, "skills", "hello"), { recursive: true });
  await mkdir(join(source, "com.obscuritylabs.colossus"));
  await cp(
    join(
      repository,
      "bundled-plugins/colossus/com.obscuritylabs.colossus/icon.png",
    ),
    join(source, "com.obscuritylabs.colossus/icon.png"),
  );
  await writeFile(
    join(source, "plugin.json"),
    JSON.stringify({
      $schema: "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
      name: "example",
      version: "1.0.0",
      description: "Scratch workspace authoring acceptance",
      extensions: {
        "com.obscuritylabs.colossus": {
          icon: "com.obscuritylabs.colossus/icon.png",
        },
      },
    }),
  );
  await writeFile(
    join(source, "skills", "hello", "SKILL.md"),
    "---\nname: hello\ndescription: Say hello using ordinary authorized tools.\n---\nSay hello from the scratch workspace.\n",
  );
  let worker: ChildProcess | undefined;
  const processes = new AcceptanceProcesses();
  const operations = new AcceptanceOperations();
  const execute = processes.execute.bind(processes);
  let scenarioFailed = false;
  let workerError = "";
  const prompts: unknown[] = [];
  let nativePaths: (string | null)[] = [];
  let consent = false;
  const invoke = async (
    command: string,
    args: Record<string, unknown>,
    overrides = {},
  ) => {
    try {
      const answer = JSON.parse(
        await execute(
          bridge,
          [join(workspace, "state.redb")],
          workspace,
          env,
          JSON.stringify({
            command,
            args,
            paths: nativePaths,
            approve: consent,
            ...overrides,
          }),
        ),
      ) as {
        response: { result?: unknown; error?: unknown };
        prompts: unknown[];
      };
      prompts.push(...answer.prompts);
      if (answer.response.error) {
        await test.info().attach("isolated-worker-diagnostic", {
          body: workerError || "No worker stderr",
          contentType: "text/plain",
        });
        throw answer.response.error;
      }
      return answer.response.result;
    } finally {
      if (command === "manage_plugin") {
        const input = args.input as
          { request?: { operation?: string } } | undefined;
        if (input?.request?.operation)
          operations.complete(input.request.operation);
      }
    }
  };
  const continueOperation = (operation: string) =>
    operations.submit(operation, () =>
      page.getByRole("button", { name: `Continue ${operation}` }).click(),
    );
  try {
    worker = processes.start(
      binary,
      ["--config", config, "--approval-mode", "ask", "worker"],
      { cwd: workspace, env, stdio: ["ignore", "ignore", "pipe"] },
    );
    worker.stderr?.on("data", (bytes: Buffer) => {
      if (workerError.length < 64 * 1024) workerError += bytes.toString();
    });
    await expect
      .poll(
        async () => {
          if (worker?.exitCode !== null || worker?.signalCode !== null)
            return workerError || "worker exited";
          try {
            await execute(
              binary,
              ["--config", config, "worker", "--status"],
              workspace,
              env,
            );
            return "ready";
          } catch (error) {
            return String(error);
          }
        },
        { timeout: 30_000 },
      )
      .toBe("ready");
    // CLI readiness alone does not prove the native adapter derived the same
    // endpoint (notably Rust versus Node canonical Windows path spellings).
    const readyInventory = (await invoke("get_plugin_inventory", {
      targetId: "local",
    })) as { plugins: { manifest: { name: string } }[] };
    expect(
      readyInventory.plugins.some(
        (plugin) => plugin.manifest.name === "colossus",
      ),
    ).toBe(true);
    await page.exposeBinding(
      "nativePluginAcceptance",
      async (_, command: string, args: Record<string, unknown>) =>
        invoke(command, args),
    );
    await page.addInitScript(() => {
      const bridgeWindow = window as unknown as {
        __TAURI_INTERNALS__: unknown;
        nativePluginAcceptance: (
          command: string,
          args: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      bridgeWindow.__TAURI_INTERNALS__ = {
        invoke: (command: string, args: Record<string, unknown>) =>
          bridgeWindow.nativePluginAcceptance(command, args),
      };
    });
    await page.goto("/?fixture=plugin-studio");
    await expect(
      page.getByRole("button", { name: /colossus 0\./u }),
    ).toBeVisible({ timeout: 15_000 });
    await page.getByRole("button", { name: /colossus 0\./u }).click();
    const detail = page.getByRole("article", { name: "colossus details" });
    for (const name of [
      "coding",
      "offline-dev",
      "security-review",
      "plugin-authoring",
    ])
      await expect(
        detail.getByRole("heading", { name: `colossus/${name}`, exact: true }),
      ).toBeVisible();
    const skill = detail.locator(".plugin-skill").filter({
      has: page.getByRole("heading", {
        name: "colossus/plugin-authoring",
        exact: true,
      }),
    });
    await skill.getByRole("button", { name: "Preview instructions" }).click();
    await expect(skill.locator("pre")).toContainText("plugin");
    await skill.getByRole("button", { name: "Browse resources" }).click();
    await expect(
      skill.getByRole("button", { name: /references\/.+\.md/u }).first(),
    ).toBeVisible();
    await skill
      .getByRole("button", { name: "Use in this conversation" })
      .click();
    await expect(
      page.getByRole("button", { name: "Remove colossus/plugin-authoring" }),
    ).toBeVisible();
    await detail.getByRole("button", { name: "Disable", exact: true }).click();
    await continueOperation("disable");
    await expect(
      detail.getByRole("button", { name: "Activate this digest" }),
    ).toBeVisible();
    // Reopening the same binary must retain the user's disabled preference.
    const listed = await execute(
      binary,
      ["--config", config, "--output", "json", "plugins", "list"],
      workspace,
      env,
    );
    expect(listed).toContain("disabled");
    await detail.getByRole("button", { name: "Activate this digest" }).click();
    await continueOperation("enable");
    await expect(
      detail.getByRole("button", { name: "Disable", exact: true }),
    ).toBeVisible();
    await expect(detail.locator(".plugin-icon img")).toHaveJSProperty(
      "naturalWidth",
      128,
    );
    await page.getByText("Developer tools", { exact: true }).click();
    nativePaths = [source];
    await page.getByRole("button", { name: "Validate", exact: true }).click();
    await continueOperation("validate");
    await expect(
      page.getByRole("status").filter({ hasText: "validate completed" }),
    ).toBeVisible();
    nativePaths = [source, join(workspace, "layout")];
    await page.getByRole("button", { name: "Package", exact: true }).click();
    await continueOperation("package");
    await expect(
      page.getByRole("status").filter({ hasText: "package completed" }),
    ).toBeVisible();
    nativePaths = [join(workspace, "layout")];
    await page.getByRole("button", { name: "Install", exact: true }).click();
    await page.getByRole("combobox", { name: "Installation source" }).click();
    await page
      .getByRole("option", { name: "OCI layout directory", exact: true })
      .click();
    await page.getByLabel("Trust profile", { exact: true }).fill("offline");
    await continueOperation("install");
    await page.getByRole("button", { name: /example 1\.0/u }).click();
    const imported = page.getByRole("article", { name: "example details" });
    await expect(imported.locator(".plugin-icon img")).toHaveJSProperty(
      "naturalWidth",
      128,
    );
    await expect(
      imported.getByRole("button", { name: "Use in this conversation" }),
    ).toBeDisabled();
    await imported
      .getByRole("button", { name: "Activate this digest" })
      .click();
    await page.getByRole("checkbox", { name: /Request approval/u }).check();
    await continueOperation("enable");
    await expect(page.getByRole("alert")).toBeVisible();
    expect(prompts.length).toBeGreaterThan(0); // The checkbox did not authorize activation.
    consent = true;
    await continueOperation("enable");
    await expect(
      imported.getByRole("button", { name: "Disable", exact: true }),
    ).toBeVisible();
    await expect(imported.locator(".plugin-icon img")).toHaveJSProperty(
      "naturalWidth",
      128,
    );
    const inventory = (await invoke("get_plugin_inventory", {
      targetId: "local",
    })) as { plugins: { manifest: { name: string }; digest: string }[] };
    const identity = inventory.plugins.find(
      (plugin) => plugin.manifest.name === "example",
    )!;
    await expect(
      invoke("manage_plugin", {
        targetId: "external",
        input: { request: { operation: "gc" } },
      }),
    ).rejects.toMatchObject({ code: "plugin_operation_failed" });
    await expect(
      invoke("read_plugin_preview", {
        targetId: "local",
        request: {
          kind: "resource",
          skillId: "example/hello",
          digest: identity.digest,
          path: "../../state.redb",
        },
      }),
    ).rejects.toMatchObject({ code: "plugin_operation_failed" });
    expect(
      await invoke(
        "manage_plugin",
        {
          targetId: "local",
          input: { request: { operation: "disable", name: "example" } },
        },
        { cancel: true },
      ),
    ).toEqual({ cancelled: true });
    await page.screenshot({
      path: "../../output/playwright/plugins-real-runtime.png",
      fullPage: true,
    });
  } catch (error) {
    scenarioFailed = true;
    await test.info().attach("isolated-worker-diagnostic", {
      body: workerError || "No worker stderr",
      contentType: "text/plain",
    });
    if (!page.isClosed())
      await test.info().attach("scenario-failure", {
        body: await page.screenshot().catch(() => Buffer.from([])),
        contentType: "image/png",
      });
    throw error;
  } finally {
    try {
      // Stop refresh callbacks before shutting down IPC, then await every owned
      // bridge/CLI/worker process's close event before touching immutable content.
      try {
        await page.close();
      } finally {
        await processes.close();
      }
      await removePrivateFixture(temporary);
    } catch (error) {
      if (!scenarioFailed) throw error;
      await test.info().attach("fixture-cleanup-diagnostic", {
        body: String(error),
        contentType: "text/plain",
      });
    }
  }
});

import type { Page } from "@playwright/test";

/** Synthetic native transport only; never captures a real window or registers a shortcut. */
export async function installDesktopExtrasFixture(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (value: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const fixture = {
      calls: [] as { command: string; args: Record<string, unknown> }[],
      revision: 0,
      delayAttach: false,
      rejectAttach: null as ((message: string) => void) | null,
      preferences: {
        schemaVersion: 1,
        appshotsEnabled: false,
        appshotShortcut: "Control+Alt+Shift+S",
        appshotIncludeText: true,
        petEnabled: false,
        petAlwaysOnTop: true,
        petMotion: true,
        petSize: "small",
        petX: null,
        petY: null,
      },
      pet: { status: "idle", motion: true, size: "small" },
      captures: {} as Record<
        string,
        {
          captureId: string;
          windowTitle: string;
          width: number;
          height: number;
          imageDataUrl: string;
          text: string | null;
        }
      >,
      emit(event: string, payload: unknown) {
        for (const id of listeners.get(event) ?? [])
          callbacks.get(id)?.({ event, id, payload });
      },
      capture(
        id: string,
        text:
          | string
          | null = "Owned fixture terminal\n$ cargo check --locked\nFinished dev profile",
      ) {
        const canvas = document.createElement("canvas");
        canvas.width = 800;
        canvas.height = 450;
        const ctx = canvas.getContext("2d")!;
        ctx.fillStyle = "#101214";
        ctx.fillRect(0, 0, 800, 450);
        ctx.fillStyle = "#96e6a1";
        ctx.font = "22px monospace";
        for (const [index, line] of (text ?? "Owned screenshot fixture")
          .split("\n")
          .entries())
          ctx.fillText(line, 24, 40 + index * 38);
        fixture.captures[id] = {
          captureId: id,
          windowTitle: "Owned Terminal Fixture",
          width: 800,
          height: 450,
          imageDataUrl: canvas.toDataURL("image/png"),
          text,
        };
        fixture.emit("appshot-started", id);
      },
    };
    const context = () => ({
      revision: fixture.revision,
      preferences: structuredClone(fixture.preferences),
      appshotsAvailable: true,
      shortcutError: null,
    });
    const artifact = (id: string, name: string, type: string) => ({
      artifactId: id,
      fileName: name,
      mediaType: type,
      sizeBytes: 100,
      sha256: "a".repeat(64),
      purpose: "run_input",
      state: "available",
      createdAt: "2026-10-29T00:00:00Z",
    });
    Object.assign(window, {
      desktopExtrasFixture: fixture,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener() {} },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback(callback: (value: unknown) => void) {
          const id = next++;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback(id: number) {
          callbacks.delete(id);
        },
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          fixture.calls.push({ command, args: structuredClone(args) });
          if (command === "plugin:event|listen") {
            const event = args.event as string;
            const id = args.handler as number;
            listeners.set(event, [...(listeners.get(event) ?? []), id]);
            return id;
          }
          if (command === "plugin:event|unlisten") {
            const event = args.event as string;
            listeners.set(
              event,
              (listeners.get(event) ?? []).filter((id) => id !== args.eventId),
            );
            return;
          }
          if (command === "desktop_features_context") return context();
          if (command === "save_desktop_features") {
            if (args.expectedRevision !== fixture.revision)
              throw {
                message:
                  "Desktop settings changed. Reopen settings before saving.",
              };
            Object.assign(fixture.preferences, args.preferences);
            fixture.revision += 1;
            fixture.emit("desktop-features-changed", null);
            return context();
          }
          if (command === "sync_pet_status") return;
          if (command === "pet_context") return structuredClone(fixture.pet);
          if (command === "pet_action") {
            if (args.action === "hide") {
              fixture.preferences.petEnabled = false;
              fixture.revision += 1;
              fixture.emit("desktop-features-changed", null);
            }
            return;
          }
          if (command === "appshot_preview")
            return structuredClone(fixture.captures[args.captureId as string]);
          if (command === "discard_appshot") {
            delete fixture.captures[args.captureId as string];
            return;
          }
          if (command === "attach_appshot" && fixture.delayAttach) {
            return new Promise((_, reject) => {
              fixture.rejectAttach = (message) => reject({ message });
            });
          }
          if (command === "attach_appshot")
            return [
              artifact(
                "fixture-appshot-image",
                "owned-capture.png",
                "image/png",
              ),
              ...(args.includeText
                ? [
                    artifact(
                      "fixture-appshot-text",
                      "owned-capture.txt",
                      "text/plain",
                    ),
                  ]
                : []),
            ];
          if (command === "list_setup_packages") return [];
          if (command === "get_managed_configuration") {
            const modulePath = "/src/components/ManagedSettingsPane.tsx";
            const { buildManagedSettingsFixture } = await import(modulePath);
            return buildManagedSettingsFixture({
              selectedSpaceId: "fixture-managed-local",
              spaces: [
                {
                  spaceId: "fixture-managed-local",
                  displayName: "Colossus",
                  displayPath: "~/Colossus",
                  archived: false,
                },
              ],
              managedModelConfiguration: {
                providers: [],
                models: [],
                roles: {},
              },
              accessProfile: "development",
              executionBoundary: "workspace_isolated",
            });
          }
          return null;
        },
      },
    });
  });
}

export async function emitAppshot(page: Page, id: string, ready = true) {
  await page.evaluate(
    ({ id, ready }) => {
      const fixture = (
        window as unknown as {
          desktopExtrasFixture: {
            capture(id: string): void;
            emit(event: string, payload: string): void;
          };
        }
      ).desktopExtrasFixture;
      fixture.capture(id);
      if (ready) fixture.emit("appshot-ready", id);
    },
    { id, ready },
  );
}

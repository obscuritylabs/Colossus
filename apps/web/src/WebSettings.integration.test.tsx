// @vitest-environment happy-dom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ControlPlaneFrame } from "@colossus/ui";
import {
  DEFAULT_THEME_PALETTES,
  parseThemePalettes,
} from "@colossus/ui/lib/palette";
import { WebSettings } from "./WebSettings";
import { AuthenticatedApp } from "./ControlPlane";
import { NavigationProvider, navigate } from "./navigation";
import type { ChatSettingsSnapshot, ChatSettingsWrite } from "./chat-settings";
import type { Me } from "./control-api";

let root: Root, container: HTMLDivElement;
let snapshot: ChatSettingsSnapshot;
const onOpenEditor = vi.fn(),
  onCloseEditor = vi.fn(),
  onDirtyChange = vi.fn(),
  onSectionChange = vi.fn();
const noop = () => {};
const appearance: ComponentProps<typeof WebSettings>["appearance"] = {
  colorTheme: "dark",
  darkPalette: "colossus",
  textSize: "comfortable",
  sendShortcut: "enter",
  palettes: parseThemePalettes(DEFAULT_THEME_PALETTES),
};
const base: ComponentProps<typeof WebSettings> = {
  project: "p",
  projectName: "Project One",
  identity: "alice",
  displayName: "Alice",
  email: "alice@example.test",
  canAdminister: true,
  canConnect: true,
  appearance,
  onAppearance: noop,
  onReturn: noop,
  onConnections: noop,
  onSaved: noop,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
  onSectionChange,
};
const me: Me = {
  user: {
    id: "alice",
    display_name: "Alice",
    email: "alice@example.test",
    active: true,
    is_admin: true,
    revision: 1,
    created_at: "",
    updated_at: "",
  },
  projects: ["p", "q"].map((id, index) => ({
    id,
    name: index ? "Project Two" : "Project One",
    description: "",
    parent_project_id: null,
    archived: false,
    revision: 1,
    created_at: "",
    updated_at: "",
  })),
  memberships: ["p", "q"].map((project_id) => ({
    subject: "alice",
    user_id: "alice",
    project_id,
    role: "project_admin",
    permissions: ["read", "execute", "control", "administer"],
  })),
};
const json = (value: unknown) => new Response(JSON.stringify(value));
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState(null, "", "/");
  localStorage.clear();
  vi.clearAllMocks();
  Object.defineProperty(window, "confirm", {
    configurable: true,
    value: () => true,
  });
  vi.spyOn(window, "matchMedia").mockImplementation((media) => ({
    matches: false,
    media,
    onchange: null,
    addListener: noop,
    removeListener: noop,
    addEventListener: noop,
    removeEventListener: noop,
    dispatchEvent: () => true,
  }));
  snapshot = {
    revision: 1,
    providers: {
      profiles: {
        company: {
          kind: "openai_responses",
          baseUrl: "https://models.example.test/v1",
          credentialReference: null,
        },
      },
    },
    models: {
      profiles: {
        fast: {
          providerProfile: "company",
          model: "model-fast",
          contextWindowTokens: 32768,
          maxOutputTokens: 4096,
        },
      },
      roles: { primary: "fast" },
    },
    mcp: { servers: {} },
    provider_authentication: { company: "personal" },
    mcp_authentication: {},
    credential_status: { providers: {}, mcp: {} },
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string, options: RequestInit) => {
      if (options.method !== "GET")
        throw new Error("This acceptance uses read-only fixtures.");
      if (path.includes("credential-store"))
        return json({ revision: 1, credentials: [], connections: [] });
      if (path.includes("variables/personal"))
        return json({ revision: 1, variables: {} });
      if (path.endsWith("provider-presets")) return json({ presets: [] });
      if (path.endsWith("chat/settings")) return json(snapshot);
      if (path.startsWith("/api/announcements"))
        return json({
          announcements: [],
          active_banner: null,
          next_cursor: null,
        });
      for (const name of ["nodes", "hosts", "tasks", "threads"])
        if (path.includes(`/${name}?`))
          return json({ [name]: [], next_cursor: null });
      throw new Error(`Unexpected read ${path}`);
    }),
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  history.replaceState(null, "", "/");
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
async function render(props: Partial<ComponentProps<typeof WebSettings>> = {}) {
  await act(() =>
    root.render(
      <WebSettings
        key={JSON.stringify([
          props.project,
          props.initialScope,
          props.initialSection,
          props.editorLocation,
        ])}
        {...base}
        {...props}
      />,
    ),
  );
}
function field(label: string) {
  return (
    [...container.querySelectorAll("label")]
      .find((item) => item.querySelector("span")?.textContent === label)
      ?.querySelector<HTMLInputElement>("input") ?? null
  );
}
function button(label: string, within: ParentNode = container) {
  const value = [...within.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) =>
      item.getAttribute("aria-label") === label ||
      item.textContent?.trim() === label,
  );
  if (!value) throw new Error(`Missing button ${label}`);
  return value;
}

it("renders Project Administration in exactly the app rail and secondary navigation without a third settings sidebar", async () => {
  await act(() =>
    root.render(
      <ControlPlaneFrame
        current="project-admin"
        navigation={[
          {
            id: "project-admin",
            label: "Project Admin",
            icon: <svg />,
            href: "/projects/p/settings/models",
          },
        ]}
        onNavigate={noop}
        secondaryNavigation={{
          id: "project:p",
          title: "Project Administration",
          context: "Project One",
          navigation: [
            {
              id: "models",
              label: "Models",
              icon: <svg />,
              href: "/projects/p/settings/models",
            },
          ],
          current: "models",
          onNavigate: noop,
        }}
      >
        <WebSettings {...base} initialScope="space" initialSection="models" />
      </ControlPlaneFrame>,
    ),
  );
  expect(container.querySelectorAll(".control-app-sidebar")).toHaveLength(1);
  expect(
    container.querySelectorAll("#control-administration-navigation"),
  ).toHaveLength(1);
  expect(container.querySelector(".settings-sidebar")).toBeNull();
  expect(container.querySelector(".managed-settings-shell")).toBeNull();
  expect(
    container.querySelector('.chat-settings[data-section="models"]'),
  ).not.toBeNull();
  expect(
    container.querySelector('.control-secondary-link[aria-current="page"]')
      ?.textContent,
  ).toBe("Models");
});

it("uses the selected project section and forwards URL editor actions rather than opening an inline form", async () => {
  await render({ initialScope: "space", initialSection: "models" });
  await act(() => button("Add model").click());
  expect(onOpenEditor).toHaveBeenCalledWith({ action: "new" });
  expect(container.querySelector(".model-editor")).toBeNull();
  await render({
    initialScope: "space",
    initialSection: "providers",
    editorLocation: { action: "edit", id: "company" },
  });
  expect(container.querySelector(".provider-editor")).not.toBeNull();
  expect(
    container.querySelector<HTMLFormElement>("form")?.textContent,
  ).toContain("Edit company");
  await act(() => button("Back to Providers").click());
  expect(onCloseEditor).toHaveBeenCalledOnce();
});

it("keeps personal settings scoped and forwards section navigation without mutating the selected view", async () => {
  await render({
    initialScope: "global",
    initialSection: "account",
    canAdminister: false,
  });
  expect(
    container.querySelector('aside[aria-label="Settings navigation"]'),
  ).not.toBeNull();
  expect(button("Personal").getAttribute("aria-pressed")).toBe("true");
  expect(container.querySelector('button[aria-label="Project"]')).toBeNull();
  expect(
    container.querySelector('[aria-label="Signed-in account"]')?.textContent,
  ).toContain("alice@example.test");
  const sections = container.querySelector(
    'nav[aria-label="Settings sections"]',
  )!;
  await act(() => button("Credentials", sections).click());
  expect(onSectionChange).toHaveBeenCalledWith("global", "credentials");
  expect(
    container.querySelector('[aria-label="Signed-in account"]'),
  ).not.toBeNull();
  expect(fetch).not.toHaveBeenCalled();
});

it("initializes a personal credential form from a direct editor location in the private project store", async () => {
  await render({
    initialScope: "global",
    initialSection: "credentials",
    editorLocation: { action: "new" },
  });
  expect(container.querySelector('input[type="password"]')).not.toBeNull();
  expect(
    container.querySelector(".settings-editor-page")?.textContent,
  ).toContain("Add credential");
  expect(
    vi
      .mocked(fetch)
      .mock.calls.some(
        ([path]) =>
          String(path) === "/api/projects/p/chat/credential-store/personal",
      ),
  ).toBe(true);
  expect(
    vi
      .mocked(fetch)
      .mock.calls.some(([path]) => String(path).includes("/chat/settings")),
  ).toBe(false);
  await act(() => button("Cancel").click());
  expect(onCloseEditor).toHaveBeenCalledOnce();
});

it("does not fetch private service settings without a project or project configuration without administration access", async () => {
  await render({
    project: "",
    initialScope: "global",
    initialSection: "variables",
  });
  expect(container.textContent).toContain("Choose a project");
  expect(fetch).not.toHaveBeenCalled();
  await render({
    initialScope: "space",
    initialSection: "providers",
    canAdminister: false,
  });
  expect(container.textContent).toContain(
    "managed by your project administrator",
  );
  expect(container.querySelector(".settings-sidebar")).toBeNull();
  expect(fetch).not.toHaveBeenCalled();
});

it("keeps the effective project and private editor draft when a project switch is rejected from an implicit-project URL", async () => {
  history.replaceState(null, "", "/settings/credentials/new");
  const confirm = vi.fn(() => false);
  vi.spyOn(window, "confirm").mockImplementation(confirm);
  await act(() =>
    root.render(
      <NavigationProvider>
        <AuthenticatedApp
          me={me}
          authConfig={null}
          settings={null}
          onSettings={noop}
          loadMe={noop}
          onSignOut={noop}
        />
      </NavigationProvider>,
    ),
  );
  const label = field("Display label")!;
  expect(label).not.toBeNull();
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(label, "Retained private draft");
    label.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(() =>
    button("Select project").dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    ),
  );
  const target = [
    ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
  ].find(
    (item) =>
      item
        .querySelector(".control-sidebar-dropdown-item-copy")
        ?.textContent?.trim() === "Project Two",
  )!;
  expect(target).toBeDefined();
  await act(() => target.click());
  expect(confirm).toHaveBeenCalledOnce();
  expect(location.pathname + location.search).toBe("/settings/credentials/new");
  expect(button("Select project").textContent).toContain("Project One");
  expect(field("Display label")?.value).toBe("Retained private draft");
  expect(
    vi
      .mocked(fetch)
      .mock.calls.some(([path]) => String(path).startsWith("/api/projects/q/")),
  ).toBe(false);
});

it("retains a canonical project save status across the inventory remount and clears it in another settings scope", async () => {
  history.replaceState(null, "", "/projects/p/settings/providers/company/edit");
  const reads = vi.mocked(fetch).getMockImplementation()!;
  let confirmSave: ((response: Response) => void) | undefined;
  let confirmReadback: ((response: Response) => void) | undefined;
  let waitingForReadback = false;
  let submitted: ChatSettingsWrite | undefined;
  vi.mocked(fetch).mockImplementation(async (input, options) => {
    const path = String(input);
    if (path === "/api/projects/p/chat/settings" && options?.method === "PUT") {
      submitted = JSON.parse(String(options.body)) as ChatSettingsWrite;
      return new Promise<Response>((resolve) => {
        confirmSave = (response) => {
          waitingForReadback = true;
          resolve(response);
        };
      });
    }
    if (
      path === "/api/projects/p/chat/settings" &&
      options?.method === "GET" &&
      waitingForReadback
    ) {
      return new Promise<Response>((resolve) => {
        confirmReadback = (response) => {
          waitingForReadback = false;
          resolve(response);
        };
      });
    }
    return reads(input, options);
  });
  await act(() =>
    root.render(
      <NavigationProvider>
        <AuthenticatedApp
          me={me}
          authConfig={null}
          settings={null}
          onSettings={noop}
          loadMe={noop}
          onSignOut={noop}
        />
      </NavigationProvider>,
    ),
  );
  const label = field("Display label")!;
  expect(label).not.toBeNull();
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(label, "Published provider");
    label.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(() =>
    container
      .querySelector("form.provider-editor")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  expect(submitted?.labels?.providers.company).toBe("Published provider");
  expect(location.pathname).toBe("/projects/p/settings/providers/company/edit");
  await act(() => confirmSave!(json({ revision: 2 })));
  expect(confirmReadback).toBeDefined();
  expect(location.pathname).toBe("/projects/p/settings/providers/company/edit");
  expect(container.textContent).not.toContain("Project configuration saved.");
  const { inheritance: _writeInheritance, ...configuration } = submitted!;
  snapshot = { ...snapshot, ...configuration, revision: 2 };
  await act(() => confirmReadback!(json(snapshot)));
  expect(location.pathname).toBe("/projects/p/settings/providers");
  expect(container.querySelector("form.provider-editor")).toBeNull();
  expect(
    [...container.querySelectorAll('[role="status"]')].filter(
      (item) => item.textContent?.trim() === "Project configuration saved.",
    ),
  ).toHaveLength(1);
  expect(container.textContent).toContain("Published provider");
  await act(() => navigate("/projects/q/settings/providers"));
  expect(container.textContent).not.toContain("Project configuration saved.");
  await act(() => navigate("/projects/p/settings/providers"));
  expect(container.textContent).not.toContain("Project configuration saved.");
});

it("keeps a personal variable save status when its URL editor returns to the private inventory", async () => {
  history.replaceState(null, "", "/settings/variables/new?project=p");
  const reads = vi.mocked(fetch).getMockImplementation()!;
  let privateVariables: Record<string, string> = {};
  vi.mocked(fetch).mockImplementation(async (input, options) => {
    if (String(input) === "/api/projects/p/chat/variables/personal") {
      if (options?.method === "PUT") {
        privateVariables = (
          JSON.parse(String(options.body)) as {
            variables: Record<string, string>;
          }
        ).variables;
        return json({ revision: 2 });
      }
      return json({
        revision: Object.keys(privateVariables).length ? 2 : 1,
        variables: privateVariables,
      });
    }
    return reads(input, options);
  });
  await act(() =>
    root.render(
      <NavigationProvider>
        <AuthenticatedApp
          me={me}
          authConfig={null}
          settings={null}
          onSettings={noop}
          loadMe={noop}
          onSignOut={noop}
        />
      </NavigationProvider>,
    ),
  );
  for (const [name, value] of [
    ["Variable name", "REGION"],
    ["Variable value", "us-east"],
  ]) {
    const input = container.querySelector<
      HTMLInputElement | HTMLTextAreaElement
    >(`[aria-label="${name}"]`)!;
    await act(() => {
      Object.getOwnPropertyDescriptor(
        input instanceof HTMLTextAreaElement
          ? HTMLTextAreaElement.prototype
          : HTMLInputElement.prototype,
        "value",
      )!.set!.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  await act(() => button("Save variable").click());
  expect(location.pathname + location.search).toBe(
    "/settings/variables?project=p",
  );
  expect(
    [...container.querySelectorAll('[role="status"]')].some(
      (item) =>
        item.textContent?.trim() === "Your project variables were saved.",
    ),
  ).toBe(true);
  expect(container.textContent).toContain("REGION");
  await act(() => navigate("/settings/account?project=p"));
  expect(container.textContent).not.toContain(
    "Your project variables were saved.",
  );
});

// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Administration } from "./Administration";
import { AdminAnnouncements } from "./AdminAnnouncements";
import { UserEditor } from "./AdminUserEditor";
import { NavigationProvider } from "./navigation";
import type { AdminSettings, AuthConfig, User } from "./control-api";
import type { Announcement } from "./announcements";
import type { SettingsEditorLocation } from "./settings-editor";

const authConfig: AuthConfig = { local_enabled: true, oidc: null };
const initialUser: User = {
  id: "outside-first-page",
  display_name: "Test operator",
  email: "operator@example.test",
  active: true,
  is_admin: false,
  revision: 2,
  created_at: "2026-10-22T12:00:00Z",
  updated_at: "2026-10-22T12:00:00Z",
  identities: [{ kind: "local", label: "Local", username: "operator" }],
};
const initialAnnouncement: Announcement = {
  id: "older-announcement",
  title: "Older update",
  summary: "A feature",
  body: "**Released feature**",
  tone: "info",
  banner: true,
  priority: 50,
  expires_at: null,
  revision: 2,
  publication_revision: 2,
  status: "published",
  published_at: "2026-10-22T12:00:00Z",
  created_at: "2026-10-22T12:00:00Z",
  updated_at: "2026-10-22T12:00:00Z",
};
let root: Root, container: HTMLDivElement;
let user: User, announcement: Announcement, settings: AdminSettings;
let detailStatus: number;
let writes: {
  path: string;
  body: Record<string, unknown>;
  resolve: (response: Response) => void;
}[];
let gets: string[];
const onSaved = vi.fn(),
  onClose = vi.fn(),
  onDirtyChange = vi.fn();
const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  vi.stubGlobal(
    "confirm",
    vi.fn(() => true),
  );
  history.replaceState(null, "", "/admin/users");
  user = { ...initialUser };
  announcement = { ...initialAnnouncement };
  settings = {
    revision: 4,
    auth: { local_enabled: true, oidc_label: null },
    classification: {
      enabled: true,
      text: "LOCAL",
      tone: "info",
      position: "top",
    },
  };
  detailStatus = 200;
  writes = [];
  gets = [];
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string, init: RequestInit) => {
      if (init.method === "GET") {
        gets.push(path);
        if (path === "/api/admin/settings") return json(settings);
        if (path === "/api/admin/announcements?limit=50")
          return json({ announcements: [], next_cursor: null });
        if (path.startsWith("/api/admin/announcements/"))
          return detailStatus === 200
            ? json({ announcement })
            : json({ error: "Detail unavailable" }, detailStatus);
        if (path.startsWith("/api/admin/users/"))
          return detailStatus === 200
            ? json({ user })
            : json({ error: "Detail unavailable" }, detailStatus);
        return json({ users: [], next_cursor: null });
      }
      return new Promise<Response>((resolve) =>
        writes.push({ path, body: JSON.parse(String(init.body)), resolve }),
      );
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
async function renderAdmin(
  editorLocation?: SettingsEditorLocation,
  view: "users" | "settings" = "users",
  configuration: AuthConfig = authConfig,
) {
  await act(() =>
    root.render(
      <NavigationProvider>
        <Administration
          authConfig={configuration}
          onRefresh={onSaved}
          onSettings={onSaved}
          view={view}
          currentUserId="admin"
          identity="admin"
          projects={[]}
          selectedProject=""
          editorLocation={editorLocation}
          onCloseEditor={onClose}
          onOpenEditor={() => {}}
          onDirtyChange={onDirtyChange}
        />
      </NavigationProvider>,
    ),
  );
}
async function renderAnnouncement() {
  await act(() =>
    root.render(
      <AdminAnnouncements
        identity="admin"
        editorLocation={{ action: "edit", id: announcement.id }}
        onCloseEditor={onClose}
        onDirtyChange={onDirtyChange}
      />,
    ),
  );
}
function button(label: string) {
  const result = [
    ...container.querySelectorAll<HTMLButtonElement>("button"),
  ].find(
    (element) =>
      element.textContent?.trim() === label ||
      element.getAttribute("aria-label") === label,
  );
  if (!result) throw new Error(`Missing button ${label}`);
  return result;
}
function field(label: string) {
  const result = [...container.querySelectorAll<HTMLLabelElement>("label")]
    .find(
      (element) => element.querySelector("span")?.textContent?.trim() === label,
    )
    ?.querySelector<HTMLInputElement | HTMLTextAreaElement>("input,textarea");
  if (!result) throw new Error(`Missing field ${label}`);
  return result;
}
async function change(label: string, value: string) {
  const input = field(label);
  await act(() => {
    const prototype =
      input instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(
      input,
      value,
    );
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function submit(formLabel?: string) {
  const form = formLabel
    ? container.querySelector(`form[aria-label="${formLabel}"]`)
    : container.querySelector("form");
  if (!form) throw new Error("Missing form");
  await act(() =>
    form.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    ),
  );
}
it("loads a direct user edit outside the paginated inventory and disables parent Back during save", async () => {
  await renderAdmin({ action: "edit", id: user.id });
  expect(gets).toEqual([`/api/admin/users/${user.id}`]);
  expect(field("Display name").value).toBe("Test operator");
  expect(container.querySelectorAll("h2")).toHaveLength(1);
  await change("Display name", "Updated operator");
  await submit("Edit user");
  expect(button("Back to user").disabled).toBe(true);
  expect(container.querySelector("fieldset")?.disabled).toBe(true);
  expect(writes[0]!.body).toEqual({
    revision: 2,
    display_name: "Updated operator",
    active: true,
    is_admin: false,
  });
  await act(() => writes[0]!.resolve(json({ user: { ...user, revision: 3 } })));
  expect(onSaved).toHaveBeenCalledOnce();
  expect(onDirtyChange).toHaveBeenLastCalledWith(false);
});
it("retains a user conflict draft until explicit current-user reload", async () => {
  await renderAdmin({ action: "edit", id: user.id });
  await change("Display name", "Proposed operator");
  await submit("Edit user");
  user = { ...user, revision: 6, display_name: "Concurrent update" };
  await act(() => writes[0]!.resolve(json({ error: "Conflict" }, 409)));
  expect(field("Display name").value).toBe("Proposed operator");
  expect(container.querySelector("fieldset")?.disabled).toBe(true);
  expect(onSaved).not.toHaveBeenCalled();
  await act(() => button("Reload current user").click());
  expect(field("Display name").value).toBe("Concurrent update");
  expect(container.querySelector("fieldset")?.disabled).toBe(false);
});
it("does not convert a missing direct user URL into a create form", async () => {
  detailStatus = 404;
  await renderAdmin({ action: "edit", id: "deleted" });
  expect(container.querySelector('form[aria-label="Create user"]')).toBeNull();
  expect(container.querySelector('form[aria-label="Edit user"]')).toBeNull();
  expect(container.querySelector('[role="alert"]')).not.toBeNull();
});
it("prevents duplicate user writes and ignores completion after unmount", async () => {
  await renderAdmin({ action: "edit", id: user.id });
  const form = container.querySelector("form")!;
  await act(() => {
    form.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    form.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
  });
  expect(writes).toHaveLength(1);
  await act(() => root.render(<p>Another page</p>));
  await act(() => writes[0]!.resolve(json({ user })));
  expect(onSaved).not.toHaveBeenCalled();
});
it("clears password entry on submission and cannot silently retry an uncertain reset", async () => {
  await act(() =>
    root.render(
      <UserEditor
        user={user}
        authConfig={authConfig}
        passwordReset
        onCancel={onClose}
        onSaved={onSaved}
      />,
    ),
  );
  await change("New password", "synthetic-password-123");
  await submit("Reset local password");
  expect(field("New password").value).toBe("");
  expect(writes[0]!.body).toEqual({
    revision: 2,
    new_password: "synthetic-password-123",
  });
  await act(() => writes[0]!.resolve(json({ error: "Unavailable" }, 503)));
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "could not be confirmed",
  );
  expect(container.querySelector("fieldset")?.disabled).toBe(true);
  expect(onSaved).not.toHaveBeenCalled();
});
it("validates a new local user's password even on a direct submit", async () => {
  await renderAdmin({ action: "new" });
  await change("Display name", "New operator");
  await change("Local username", "new-operator");
  await change("Local password", "short");
  await submit("Create user");
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "15 characters",
  );
  expect(writes).toEqual([]);
});
it.each([
  ["Display name", "é".repeat(129), "display name"],
  ["Display name", "Invalid\u0000name", "display name"],
  ["Email · optional", "x".repeat(309) + "@example.test", "email address"],
  ["Local username", "x", "local username"],
  ["Local username", "has space", "local username"],
  ["Local password", "🙂".repeat(8), "15 characters"],
  ["Local password", "界".repeat(342), "1,024 UTF-8 bytes"],
])("explains invalid user %s before a write", async (label, value, message) => {
  await renderAdmin({ action: "new" });
  await change("Display name", "New operator");
  await change("Local username", "new-operator");
  await change("Local password", "Synthetic-only-password");
  await change(label, value);
  await submit("Create user");
  expect(writes).toEqual([]);
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    message,
  );
});
it.each(["LOCAL\u0000MARKING", "x".repeat(161)])(
  "explains invalid classification markings before publishing",
  async (value) => {
    await renderAdmin(undefined, "settings");
    await change("Classification text", value);
    await submit();
    expect(writes).toEqual([]);
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      "160 classification characters without control characters",
    );
  },
);

it.each(["local", "oidc", "both"] as const)(
  "accepts bounded user metadata with %s login identities and preserves exact OIDC subjects",
  async (kind) => {
    const configuration: AuthConfig = {
      local_enabled: kind !== "oidc",
      oidc:
        kind !== "local"
          ? { label: "Organization", login_url: "/api/auth/oidc/login" }
          : null,
    };
    const subject = " " + "é".repeat(127) + " ";
    const name = "n".repeat(256);
    await renderAdmin({ action: "new" }, "users", configuration);
    expect(field("Display name").maxLength).toBe(256);
    await change("Display name", name);
    if (kind !== "oidc") {
      await change("Local username", "new-operator");
      await change("Local password", "Synthetic-only-password");
    }
    if (kind !== "local") await change("OIDC subject", subject);
    await submit("Create user");
    expect(writes).toHaveLength(1);
    expect(writes[0]!.body).toMatchObject({ display_name: name });
    if (kind === "oidc") {
      expect(writes[0]!.body).not.toHaveProperty("username");
      expect(writes[0]!.body).not.toHaveProperty("password");
    } else
      expect(writes[0]!.body).toMatchObject({
        username: "new-operator",
        password: "Synthetic-only-password",
      });
    if (kind === "local")
      expect(writes[0]!.body).not.toHaveProperty("oidc_subject");
    else expect(writes[0]!.body.oidc_subject).toBe(subject);
    await act(() => writes[0]!.resolve(json({ user })));
  },
);

it.each(["é".repeat(129), "invalid\u0000subject", "invalid\u0085subject"])(
  "explains invalid OIDC subjects before a write",
  async (subject) => {
    await renderAdmin({ action: "new" }, "users", {
      local_enabled: false,
      oidc: { label: "Organization", login_url: "/api/auth/oidc/login" },
    });
    await change("Display name", "OIDC operator");
    await change("OIDC subject", subject);
    await submit("Create user");
    expect(writes).toEqual([]);
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      "exact OIDC subject of 1–256 UTF-8 bytes without control characters",
    );
  },
);

it("accepts 160 Unicode classification characters without a UTF-16 truncation cap", async () => {
  await renderAdmin(undefined, "settings");
  const text = "🙂".repeat(160);
  expect(field("Classification text").maxLength).toBe(320);
  await change("Classification text", text);
  await submit();
  expect(writes).toHaveLength(1);
  expect(writes[0]!.body.classification).toMatchObject({ text });
  await act(() =>
    writes[0]!.resolve(
      json({
        ...settings,
        classification: { ...settings.classification, text },
      }),
    ),
  );
});

it("loads a linked announcement independently of the first inventory page", async () => {
  await renderAnnouncement();
  expect(gets).toContain(`/api/admin/announcements/${announcement.id}`);
  expect(field("Title").value).toBe("Older update");
  expect(
    container.querySelector('[aria-label="Instance announcements"]'),
  ).toBeNull();
  expect(container.querySelectorAll("h2")).toHaveLength(1);
  await change("Title", "Updated notice");
  await act(() => button("Publish changes").click());
  expect(button("Back to announcements").disabled).toBe(true);
  expect(writes[0]!.body.revision).toBe(2);
  await act(() =>
    writes[0]!.resolve(
      json({
        announcement: { ...announcement, title: "Updated notice", revision: 3 },
      }),
    ),
  );
  expect(onClose).toHaveBeenCalledOnce();
});
it("keeps an announcement proposal through a failed explicit conflict reload", async () => {
  await renderAnnouncement();
  await change("Title", "Proposed notice");
  await act(() => button("Publish changes").click());
  await act(() => writes[0]!.resolve(json({ error: "Conflict" }, 409)));
  detailStatus = 503;
  await act(() => button("Reload current version").click());
  expect(field("Title").value).toBe("Proposed notice");
  expect(button("Publish changes").disabled).toBe(true);
  expect(onClose).not.toHaveBeenCalled();
});
it("guards an in-flight pristine announcement archive and releases busy state", async () => {
  await renderAnnouncement();
  await act(() => button("Archive announcement").click());
  expect(onDirtyChange).toHaveBeenLastCalledWith(true);
  expect(button("Back to announcements").disabled).toBe(true);
  await act(() => writes[0]!.resolve(json({ error: "Unavailable" }, 503)));
  expect(button("Back to announcements").disabled).toBe(false);
  expect(button("Retry this exact save")).toBeDefined();
  expect(onDirtyChange).toHaveBeenLastCalledWith(true);
});
it("fences classification changes and reloads an explicit new revision after conflict", async () => {
  await renderAdmin(undefined, "settings");
  await change("Classification text", "PROPOSED");
  await submit();
  expect(writes[0]!.body).toEqual({
    revision: 4,
    classification: { ...settings.classification, text: "PROPOSED" },
  });
  expect(container.querySelector("fieldset")?.disabled).toBe(true);
  settings = {
    ...settings,
    revision: 9,
    classification: { ...settings.classification, text: "CURRENT" },
  };
  await act(() => writes[0]!.resolve(json({ error: "Conflict" }, 409)));
  expect(field("Classification text").value).toBe("PROPOSED");
  expect(container.querySelector("fieldset")?.disabled).toBe(true);
  await act(() => button("Reload current settings").click());
  expect(field("Classification text").value).toBe("CURRENT");
  expect(container.querySelector("fieldset")?.disabled).toBe(false);
});

it("uses the actual classification PATCH response without requiring authentication metadata", async () => {
  await renderAdmin(undefined, "settings");
  await change("Classification text", "PUBLISHED");
  await submit();
  const published = {
    revision: 5,
    classification: { ...settings.classification, text: "PUBLISHED" },
  };
  settings = { ...settings, ...published };
  await act(() => writes[0]!.resolve(json(published)));
  expect(onSaved).toHaveBeenCalledWith(published);
  expect(field("Classification text").value).toBe("PUBLISHED");
  expect(container.querySelector('[role="status"]')?.textContent).toContain(
    "saved",
  );
});

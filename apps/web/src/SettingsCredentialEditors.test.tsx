// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  CredentialStoreSettings,
  type CredentialSnapshot,
} from "./CredentialStoreSettings";
import { PersonalVariables } from "./PersonalVariables";
import type { SettingsEditorLocation } from "./settings-editor";
import type { CredentialExecution } from "./credential-types";

let root: Root, container: HTMLDivElement;
let credentials: CredentialSnapshot;
let variables: {
  revision: number;
  variables: Record<string, string>;
  execution?: CredentialExecution;
};
let writes: { path: string; body: Record<string, unknown> }[];
let complete: ((response: Response) => void) | undefined;
const onOpenEditor = vi.fn(),
  onCloseEditor = vi.fn(),
  onDirtyChange = vi.fn(),
  onSaved = vi.fn();
const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status });
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  writes = [];
  complete = undefined;
  credentials = {
    revision: 3,
    credentials: [
      {
        id: "key",
        label: "Saved API key",
        kind: "api_key",
        legacy: false,
        used_by: [],
      },
    ],
    connections: [],
  };
  variables = { revision: 2, variables: { REGION: "old-region" } };
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string, init: RequestInit) => {
      if (init.method === "GET")
        return json(path.includes("variables") ? variables : credentials);
      writes.push({ path, body: JSON.parse(String(init.body)) });
      return new Promise<Response>((resolve) => {
        complete = resolve;
      });
    }),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function renderSecret(editorLocation?: SettingsEditorLocation) {
  await act(() =>
    root.render(
      <CredentialStoreSettings
        path="/api/secrets"
        identity="alice"
        canEdit
        onSaved={onSaved}
        editorLocation={editorLocation}
        onOpenEditor={onOpenEditor}
        onCloseEditor={onCloseEditor}
        onDirtyChange={onDirtyChange}
      />,
    ),
  );
}
async function renderVariable(editorLocation?: SettingsEditorLocation) {
  await act(() =>
    root.render(
      <PersonalVariables
        project="p"
        identity="alice"
        canEdit
        editorLocation={editorLocation}
        onOpenEditor={onOpenEditor}
        onCloseEditor={onCloseEditor}
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
  if (!result) throw new Error(`Missing button: ${label}`);
  return result;
}
async function change(label: string, value: string) {
  const input = container.querySelector<HTMLInputElement | HTMLTextAreaElement>(
    `[aria-label="${label}"]`,
  );
  if (!input) throw new Error(`Missing field: ${label}`);
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
async function submitSecret() {
  const input = container.querySelector<HTMLInputElement>(
    'input[type="password"]',
  )!;
  input.value = "synthetic-write-only-token";
  await act(() => {
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input
      .closest("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  });
  expect(input.value).toBe("");
}
it("navigates inventories into dedicated new and rotation routes", async () => {
  await renderSecret();
  await act(() => button("Add credential").click());
  expect(onOpenEditor).toHaveBeenLastCalledWith({ action: "new" });
  expect(container.querySelector('input[type="password"]')).toBeNull();
  await act(() => button("Rotate credential Saved API key").click());
  expect(onOpenEditor).toHaveBeenLastCalledWith({ action: "edit", id: "key" });
});
it("opens a direct rotation URL with write-only secure entry and preserves assignments", async () => {
  await renderSecret({ action: "edit", id: "key" });
  expect(
    [...container.querySelectorAll<HTMLInputElement>("input")].some(
      (input) => input.value === "Saved API key",
    ),
  ).toBe(true);
  expect(container.textContent).toContain(
    "Existing connection assignments stay in place",
  );
  expect(
    container.querySelector('[aria-label="Credential connection"]'),
  ).toBeNull();
  expect(
    container.querySelector('[aria-label="Search credentials"]'),
  ).toBeNull();
  await submitSecret();
  expect(writes).toEqual([
    {
      path: "/api/secrets/key",
      body: {
        revision: 3,
        label: "Saved API key",
        kind: "api_key",
        secret: "synthetic-write-only-token",
      },
    },
  ]);
  credentials = { ...credentials, revision: 4 };
  await act(() => complete!(json({ revision: 4 })));
  expect(onCloseEditor).toHaveBeenCalledOnce();
  expect(onSaved).toHaveBeenCalledOnce();
  expect(container.innerHTML).not.toContain("synthetic-write-only-token");
});
it("does not turn a missing credential edit URL into creation", async () => {
  await renderSecret({ action: "edit", id: "deleted" });
  expect(container.textContent).toContain("Setting unavailable");
  expect(container.querySelector('input[type="password"]')).toBeNull();
  expect(writes).toEqual([]);
});
it("locks stale credential writes even after metadata refresh", async () => {
  await renderSecret({ action: "edit", id: "key" });
  await submitSecret();
  credentials = { ...credentials, revision: 8 };
  await act(() =>
    complete!(json({ error: "The credential revision changed." }, 409)),
  );
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "reload",
  );
  expect(button("Save rotated credential").disabled).toBe(true);
  expect(onCloseEditor).not.toHaveBeenCalled();
  expect(onSaved).not.toHaveBeenCalled();
});
it("ignores a previous secret save completion after another editor route opens", async () => {
  await renderSecret({ action: "edit", id: "key" });
  await submitSecret();
  await renderSecret({ action: "new" });
  await act(() => complete!(json({ revision: 4 })));
  expect(onCloseEditor).not.toHaveBeenCalled();
  expect(onSaved).not.toHaveBeenCalled();
  expect(container.querySelector('input[type="password"]')).not.toBeNull();
});
it("opens a personal variable by URL and saves only its reviewed revision", async () => {
  await renderVariable({ action: "edit", id: "REGION" });
  const name = container.querySelector<HTMLInputElement>(
    '[aria-label="Variable name"]',
  )!;
  expect(name.value).toBe("REGION");
  expect(name.readOnly).toBe(true);
  expect(
    container.querySelector('[aria-label="Your project variables"]'),
  ).toBeNull();
  await change("Variable value", "new-region\nsecond-line");
  expect(onDirtyChange).toHaveBeenLastCalledWith(true);
  await act(() => button("Save variable").click());
  expect(writes).toEqual([
    {
      path: "/api/projects/p/chat/variables/personal",
      body: { revision: 2, variables: { REGION: "new-region\nsecond-line" } },
    },
  ]);
  variables = { revision: 3, variables: { REGION: "new-region\nsecond-line" } };
  await act(() => complete!(json({ revision: 3 })));
  expect(onCloseEditor).toHaveBeenCalledOnce();
});
it("validates personal variable UTF-8 bytes and preserves special own keys", async () => {
  await renderVariable({ action: "new" });
  await change("Variable name", "__proto__");
  await change("Variable value", "🙂".repeat(1025));
  await act(() => button("Save variable").click());
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "4,096 UTF-8 bytes",
  );
  expect(writes).toEqual([]);
  await change("Variable value", "valid");
  await act(() => button("Save variable").click());
  expect(Object.hasOwn(writes[0]!.body.variables as object, "__proto__")).toBe(
    true,
  );
  await act(() => complete!(json({ revision: 3 })));
});
it("does not create a missing variable from a stale edit URL", async () => {
  await renderVariable({ action: "edit", id: "deleted" });
  expect(container.textContent).toContain("Setting unavailable");
  expect(container.querySelector('[aria-label="Variable value"]')).toBeNull();
});
it("keeps personal variable conflicts locked after refreshing current values", async () => {
  await renderVariable({ action: "edit", id: "REGION" });
  await change("Variable value", "proposed-region");
  await act(() => button("Save variable").click());
  variables = { revision: 7, variables: { REGION: "another-edit" } };
  await act(() =>
    complete!(json({ error: "The variables revision changed." }, 409)),
  );
  expect(button("Save variable").disabled).toBe(true);
  expect(
    container.querySelector<HTMLTextAreaElement>(
      '[aria-label="Variable value"]',
    )?.value,
  ).toBe("proposed-region");
  expect(onCloseEditor).not.toHaveBeenCalled();
});
it("ignores a personal variable completion after moving to a different URL", async () => {
  await renderVariable({ action: "edit", id: "REGION" });
  await change("Variable value", "new-region");
  await act(() => button("Save variable").click());
  await renderVariable({ action: "new" });
  await act(() => complete!(json({ revision: 3 })));
  expect(onCloseEditor).not.toHaveBeenCalled();
  expect(
    container.querySelector<HTMLInputElement>('[aria-label="Variable name"]')
      ?.value,
  ).toBe("");
});
it("does not submit personal variable updates twice in the same turn", async () => {
  await renderVariable({ action: "edit", id: "REGION" });
  await change("Variable value", "new-region");
  const form = container.querySelector(
    'form[aria-label="Edit personal variable"]',
  )!;
  await act(() => {
    form.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    form.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
  });
  expect(writes).toHaveLength(1);
  await act(() => complete!(json({ revision: 3 })));
});

it("keeps credential metadata read-only before the operator enables a runtime", async () => {
  credentials = {
    revision: 0,
    credentials: [],
    connections: [],
    execution: {
      enabled: false,
      settings_editable: false,
      reason: "An operator must deploy the hosted runtime.",
    },
  };
  await renderSecret();
  expect(button("Add credential").disabled).toBe(true);
  expect(container.textContent).toContain(
    "An operator must deploy the hosted runtime.",
  );
  await renderSecret({ action: "edit", id: "unavailable" });
  expect(container.textContent).toContain(
    "An operator must deploy the hosted runtime.",
  );
  expect(container.querySelector('input[type="password"]')).toBeNull();
  expect(writes).toEqual([]);
});
it("keeps personal variables read-only before the operator enables a runtime", async () => {
  variables = {
    revision: 0,
    variables: {},
    execution: {
      enabled: false,
      settings_editable: false,
      reason: "An operator must deploy the hosted runtime.",
    },
  };
  await renderVariable();
  expect(button("Add variable").disabled).toBe(true);
  await renderVariable({ action: "new" });
  expect(container.textContent).toContain(
    "An operator must deploy the hosted runtime.",
  );
  expect(container.querySelector('[aria-label="Variable value"]')).toBeNull();
  expect(writes).toEqual([]);
});

it("fences credential completion through an A to B to A editor transition", async () => {
  await renderSecret({ action: "edit", id: "key" });
  await submitSecret();
  await renderSecret({ action: "new" });
  await renderSecret({ action: "edit", id: "key" });
  const freshInput = container.querySelector<HTMLInputElement>(
    'input[type="password"]',
  )!;
  freshInput.value = "new-private-draft";
  credentials = { ...credentials, revision: 4 };
  await act(() => complete!(json({ revision: 4 })));
  expect(onCloseEditor).not.toHaveBeenCalled();
  expect(onSaved).not.toHaveBeenCalled();
  expect(
    container.querySelector<HTMLInputElement>('input[type="password"]')?.value,
  ).toBe("new-private-draft");
});
it("fences personal variable completion through an A to B to A editor transition", async () => {
  await renderVariable({ action: "edit", id: "REGION" });
  await change("Variable value", "first-proposal");
  await act(() => button("Save variable").click());
  await renderVariable({ action: "new" });
  await renderVariable({ action: "edit", id: "REGION" });
  await change("Variable value", "newer-proposal");
  variables = { ...variables, revision: 3 };
  await act(() => complete!(json({ revision: 3 })));
  expect(onCloseEditor).not.toHaveBeenCalled();
  expect(
    container.querySelector<HTMLTextAreaElement>(
      '[aria-label="Variable value"]',
    )?.value,
  ).toBe("newer-proposal");
});

it("opens a shareable connection-targeted credential page with the exact reviewed destination", async () => {
  credentials.connections = [
    {
      kind: "mcp",
      id: "docs",
      label: "Docs",
      endpoint: "https://docs.example/mcp",
      credential_binding: "exact-reviewed-binding",
      credential_id: null,
    },
  ];
  await renderSecret({ action: "new", target: { kind: "mcp", id: "docs" } });
  expect(container.textContent).toContain("https://docs.example/mcp");
  const label = [...container.querySelectorAll("label")]
    .find((row) => row.querySelector("span")?.textContent === "Display label")
    ?.querySelector("input")!;
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(label, "Docs credential");
    label.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await submitSecret();
  expect(writes[0]!.body.attachment).toEqual({
    kind: "mcp",
    connection: "docs",
    credential_binding: "exact-reviewed-binding",
  });
  await act(() => complete!(json({ revision: 4 })));
});
it("does not create an unattached secret when a URL destination is unavailable", async () => {
  await renderSecret({ action: "new", target: { kind: "mcp", id: "removed" } });
  expect(container.textContent).toContain("connection is no longer available");
  expect(container.querySelector('input[type="password"]')).toBeNull();
});

// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PolicyEditor, ProjectAccess, ProjectEditor } from "./Projects";
import type { PolicyBaseline, Project } from "./control-api";

const resources = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("./resources", () => ({
  useResource: (path: string | null) => resources.read(path),
  errorMessage: (error: unknown) =>
    error instanceof Error ? error.message : "Could not save.",
}));

let root: Root, container: HTMLDivElement;
const refresh = vi.fn();
const project: Project = {
  id: "p",
  name: "Original name",
  description: "",
  parent_project_id: null,
  archived: false,
  revision: 1,
  created_at: "",
  updated_at: "",
};
const baseline: PolicyBaseline = {
  revision: 1,
  required_sandbox_profile: "default",
  allowed_approval_modes: [],
  allowed_tools: null,
};
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  refresh.mockReset();
  resources.read.mockReset();
  resources.read.mockImplementation(() => ({
    data: null,
    error: "",
    loading: false,
    refresh,
  }));
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
function projectField(label: string) {
  const field = [...container.querySelectorAll("label")]
    .find((element) => element.querySelector("span")?.textContent === label)
    ?.querySelector<HTMLInputElement | HTMLTextAreaElement>("input,textarea");
  if (!field) throw new Error(`Missing project field ${label}`);
  return field;
}
async function input(
  node: HTMLInputElement | HTMLTextAreaElement,
  value: string,
) {
  await act(() => {
    const prototype =
      node instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(node, value);
    node.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function submitTwice() {
  await act(() => {
    const form = container.querySelector("form")!;
    form.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    form.dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
  });
}
function pendingResponse() {
  let resolve!: (response: Response) => void;
  const promise = new Promise<Response>((done) => {
    resolve = done;
  });
  const fetch = vi.fn(
    (_input: RequestInfo | URL, _init?: RequestInit) => promise,
  );
  vi.stubGlobal("fetch", fetch);
  return {
    fetch,
    resolve: async (body: unknown) => {
      await act(async () =>
        resolve(
          new Response(JSON.stringify(body), {
            status: 200,
            headers: { "Content-Type": "application/json" },
          }),
        ),
      );
    },
  };
}
function unloadIsBlocked() {
  const event = new Event("beforeunload", { cancelable: true });
  window.dispatchEvent(event);
  return event.defaultPrevented;
}

it.each([
  ["name", "é".repeat(129), "project name"],
  ["name", "Invalid\u0000name", "project name"],
  ["description", "界".repeat(1366), "description"],
  ["description", "Invalid\u0000description", "description"],
])(
  "explains invalid project %s values before a write",
  async (field, value, message) => {
    const response = pendingResponse();
    await act(() =>
      root.render(
        <ProjectEditor
          project={project}
          projects={[project]}
          onSaved={() => {}}
        />,
      ),
    );
    await input(projectField(field === "name" ? "Name" : "Description"), value);
    await submitTwice();
    expect(response.fetch).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      message,
    );
  },
);

it.each([
  ["n".repeat(256), "d".repeat(4096)],
  ["é".repeat(128), "界".repeat(1365) + "x"],
])(
  "accepts project metadata at its UTF-8 byte boundaries",
  async (name, description) => {
    const response = pendingResponse();
    await act(() =>
      root.render(
        <ProjectEditor
          project={project}
          projects={[project]}
          onSaved={() => {}}
        />,
      ),
    );
    expect(projectField("Name").maxLength).toBe(256);
    expect(projectField("Description").maxLength).toBe(4096);
    await input(projectField("Name"), name);
    await input(projectField("Description"), description);
    await submitTwice();
    expect(response.fetch).toHaveBeenCalledTimes(1);
    expect(
      JSON.parse(response.fetch.mock.calls[0]![1]!.body as string),
    ).toMatchObject({
      revision: 1,
      name,
      description,
    });
    await response.resolve({
      project: { ...project, name, description, revision: 2 },
    });
  },
);

it("preserves a project draft and its reviewed revision across identity refreshes, locks submission, and clears the guard after saving", async () => {
  const response = pendingResponse(),
    onSaved = vi.fn();
  await act(() =>
    root.render(
      <ProjectEditor
        project={project}
        projects={[project]}
        onSaved={onSaved}
      />,
    ),
  );
  const name = projectField("Name");
  await input(name, "Reviewed draft");
  expect(unloadIsBlocked()).toBe(true);
  const newer = { ...project, revision: 2, name: "Changed elsewhere" };
  await act(() =>
    root.render(
      <ProjectEditor project={newer} projects={[newer]} onSaved={onSaved} />,
    ),
  );
  expect(name.value).toBe("Reviewed draft");
  await submitTwice();
  expect(response.fetch).toHaveBeenCalledTimes(1);
  expect(
    JSON.parse(response.fetch.mock.calls[0]![1]!.body as string),
  ).toMatchObject({ revision: 1, name: "Reviewed draft" });
  expect(name.disabled).toBe(true);
  await response.resolve({
    project: { ...newer, revision: 3, name: "Reviewed draft" },
  });
  expect(onSaved).toHaveBeenCalledTimes(1);
  expect(unloadIsBlocked()).toBe(false);
});

it("retains the policy draft and original CAS revision on resource refresh, with all controls disabled during one save", async () => {
  let current = baseline;
  resources.read.mockImplementation(() => ({
    data: current,
    error: "",
    loading: false,
    refresh,
  }));
  const response = pendingResponse();
  await act(() => root.render(<PolicyEditor project="p" canManage />));
  const profile = container.querySelector<HTMLInputElement>(
    'input[placeholder="Any profile"]',
  )!;
  await input(profile, "reviewed-profile");
  expect(unloadIsBlocked()).toBe(true);
  current = {
    ...baseline,
    revision: 2,
    required_sandbox_profile: "changed-elsewhere",
  };
  await act(() => root.render(<PolicyEditor project="p" canManage />));
  expect(profile.value).toBe("reviewed-profile");
  await submitTwice();
  expect(response.fetch).toHaveBeenCalledTimes(1);
  expect(
    JSON.parse(response.fetch.mock.calls[0]![1]!.body as string),
  ).toMatchObject({
    revision: 1,
    required_sandbox_profile: "reviewed-profile",
  });
  expect(
    [...container.querySelectorAll<HTMLInputElement>("form input")].every(
      (field) => field.disabled,
    ),
  ).toBe(true);
  await response.resolve({
    ...current,
    revision: 3,
    required_sandbox_profile: "reviewed-profile",
  });
  expect(container.querySelector('[role="status"]')?.textContent).toContain(
    "Monitoring baseline saved",
  );
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(unloadIsBlocked()).toBe(false);
});

it("ignores an old policy save after switching to another project and returning", async () => {
  resources.read.mockImplementation(() => ({
    data: baseline,
    error: "",
    loading: false,
    refresh,
  }));
  const response = pendingResponse();
  await act(() => root.render(<PolicyEditor project="p" canManage />));
  await input(
    container.querySelector<HTMLInputElement>(
      'input[placeholder="Any profile"]',
    )!,
    "old draft",
  );
  await submitTwice();
  await act(() => root.render(<PolicyEditor project="q" canManage />));
  await act(() => root.render(<PolicyEditor project="p" canManage />));
  const profile = container.querySelector<HTMLInputElement>(
    'input[placeholder="Any profile"]',
  )!;
  await input(profile, "current draft");
  await response.resolve({
    ...baseline,
    revision: 2,
    required_sandbox_profile: "old draft",
  });
  expect(profile.value).toBe("current draft");
  expect(container.querySelector('[role="status"]')).toBeNull();
  expect(refresh).not.toHaveBeenCalled();
  expect(unloadIsBlocked()).toBe(true);
});

it("locks every membership form control and ignores a response from a previous project scope", async () => {
  const candidates = { users: [{ id: "bob", display_name: "Bob" }] };
  const members = {
    members: [
      {
        project_id: "p",
        user_id: "bob",
        display_name: "Bob",
        role: "viewer",
        revision: 1,
      },
    ],
    next_cursor: null,
  };
  resources.read.mockImplementation((path: string | null) => ({
    data: path?.includes("member-candidates")
      ? candidates
      : path
        ? members
        : null,
    error: "",
    loading: false,
    refresh,
  }));
  const response = pendingResponse(),
    onChanged = vi.fn();
  await act(() =>
    root.render(<ProjectAccess project="p" canManage onChanged={onChanged} />),
  );
  const rowRole = container.querySelector<HTMLButtonElement>(
    '[role="combobox"][aria-label="Role for Bob"]',
  )!;
  await act(() => rowRole.click());
  const option = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((node) => node.textContent?.trim() === "Operator")!;
  await act(() => option.click());
  expect(response.fetch).toHaveBeenCalledTimes(1);
  expect(
    container.querySelector<HTMLInputElement>(
      'input[placeholder="Type at least two characters"]',
    )?.disabled,
  ).toBe(true);
  expect(
    [
      ...container.querySelectorAll<HTMLButtonElement>(
        '.member-invite [role="combobox"]',
      ),
    ].every((field) => field.disabled),
  ).toBe(true);
  await act(() =>
    root.render(<ProjectAccess project="q" canManage onChanged={onChanged} />),
  );
  await act(() =>
    root.render(<ProjectAccess project="p" canManage onChanged={onChanged} />),
  );
  const query = container.querySelector<HTMLInputElement>(
    'input[placeholder="Type at least two characters"]',
  )!;
  await input(query, "Current search");
  await response.resolve({});
  expect(query.value).toBe("Current search");
  expect(onChanged).not.toHaveBeenCalled();
  expect(refresh).not.toHaveBeenCalled();
});

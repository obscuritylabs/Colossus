// @vitest-environment happy-dom
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ThreadSessionViews, sessionRunViews } from "./ThreadSessionViews";
import type { Task, Update } from "./api";
import { SessionSourcesView } from "@colossus/ui/session/SessionWorkspace";

const props = {
  tasks: [] as Task[],
  updates: [] as Update[],
  onChangeView: vi.fn(),
  onRevisePlan: vi.fn(),
  canContinuePlan: true,
};
async function withRendered(
  content: ReactNode,
  assertion: (container: HTMLDivElement, root: Root) => void | Promise<void>,
) {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(() => root.render(content));
    await assertion(container, root);
  } finally {
    await act(() => root.unmount());
    container.remove();
  }
}
function task(output: string): Task {
  return {
    task_id: "task",
    project_id: "project",
    node_id: "node",
    subject: "user",
    request: {
      input: [{ text: "Review released records" }],
      mode: "plan",
      role: "primary",
    },
    run_id: "run",
    last_sequence: 1,
    revision: 1,
    snapshot: {
      run: {
        run_id: "run",
        session_id: "session",
        title: "Released plan",
        status: "completed",
        mode: "plan",
        role: "primary",
        last_sequence: 1,
        created_at: "2026-10-08T12:00:00Z",
        updated_at: "2026-10-08T12:01:00Z",
        terminal: { result: { output, plan_id: "plan", plan_revision: 2 } },
      },
      pending_interactions: [],
    },
  };
}

it("keeps unavailable topology controls absent and snapshot coverage unknown", async () => {
  await withRendered(
    <ThreadSessionViews {...props} view="topology" />,
    async (container, root) => {
      expect(container.textContent).toContain("Session map unavailable");
      expect(container.textContent).toContain(
        "does not release its local session map",
      );
      expect(container.querySelector("button")).toBeNull();
      expect(container.textContent).not.toContain("Layers");
      expect(container.textContent).not.toContain("Collapse all");
      await act(() =>
        root.render(<ThreadSessionViews {...props} view="snapshots" />),
      );
      expect(
        container.querySelector('[aria-label="Snapshot count unavailable"]')
          ?.textContent,
      ).toBe("—");
      expect(container.textContent).toContain(
        "Context snapshots are unavailable",
      );
      expect(container.textContent).not.toContain("No context snapshots");
    },
  );
});

it("keeps a plan with absent status readable without treating it as a draft", async () => {
  await withRendered(
    <ThreadSessionViews
      {...props}
      view="plans"
      tasks={[task("Saved plan preview")]}
    />,
    (container) => {
      expect(container.textContent).toContain("Status not reported");
      expect(container.textContent).not.toContain("Draft");
      const revise = [
        ...container.querySelectorAll<HTMLButtonElement>("button"),
      ].find((button) => button.textContent === "Revise");
      expect(revise?.disabled).toBe(true);
      expect(container.textContent).toContain("Saved plan preview");
    },
  );
});

it("routes released source URLs through the Web host link adapter", async () => {
  const output =
    "## Sources\n\n- [R1] Public documentation — https://example.test/docs\n- [R2] Disallowed credentials — https://user:password@example.test/private\n- [R3] Local file — repo://docs/runtime.md";
  await withRendered(
    <ThreadSessionViews {...props} view="sources" tasks={[task(output)]} />,
    (container) => {
      const links = [...container.querySelectorAll<HTMLAnchorElement>("a")];
      expect(links.map((link) => link.getAttribute("href"))).toEqual([
        "https://example.test/docs",
      ]);
      expect(links[0]?.getAttribute("rel")).toBe("noopener noreferrer");
      expect(links[0]?.getAttribute("target")).toBe("_blank");
      expect(container.textContent).toContain("repo://docs/runtime.md");
      expect(container.textContent).not.toContain("Open file");
    },
  );
});

it("keeps unadapted source links inert and opens only a released safe workspace path", async () => {
  const openFile = vi.fn();
  const views = sessionRunViews(
    [
      task(
        "## Sources\n\n- [R1] Documentation — https://example.test/docs\n- [R2] Workspace evidence — repo://docs/runtime.md#evidence\n- [R3] Traversal — repo://../private.txt",
      ),
    ],
    [],
  );
  await withRendered(
    <SessionSourcesView views={views} onOpenWorkspaceFile={openFile} />,
    async (container) => {
      expect(container.querySelector("a")).toBeNull();
      const files = [
        ...container.querySelectorAll<HTMLButtonElement>("button"),
      ].filter((button) => button.textContent === "Open file");
      expect(files).toHaveLength(1);
      await act(() => files[0]!.click());
      expect(openFile).toHaveBeenCalledExactlyOnceWith("docs/runtime.md");
    },
  );
});

it("distinguishes incomplete artifact coverage and opens only released metadata", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onInspectArtifact = vi.fn();
  const props = {
    onInspectArtifact,
    view: "resources" as const,
    tasks: [],
    updates: [] as Update[],
    onChangeView: vi.fn(),
    onRevisePlan: vi.fn(),
    canContinuePlan: false,
  };
  try {
    await act(() => root.render(<ThreadSessionViews {...props} />));
    expect(container.textContent).toContain(
      "No artifact metadata in the loaded messages.",
    );
    expect(container.textContent).not.toContain(
      "No artifacts have been released.",
    );
    expect(container.textContent).toContain(
      "Local files and earlier history may be available in Desktop.",
    );
    const event: Update = {
      run_id: "run",
      sequence: 1,
      created_at: "2026-10-08T12:00:00Z",
      update: {
        message: {
          content: [
            {
              artifact: {
                artifact_id: "artifact-one",
                file_name: "report.json",
                media_type: "application/json",
                byte_length: 44,
                state: "released",
              },
            },
          ],
        },
      },
    };
    await act(() =>
      root.render(<ThreadSessionViews {...props} updates={[event]} />),
    );
    const button = [...container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("report.json"),
    );
    expect(button).toBeDefined();
    await act(() => button!.click());
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(onInspectArtifact).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({
        fileName: "report.json",
        mediaType: "application/json",
        sizeLabel: "44 bytes",
      }),
    );
  } finally {
    await act(() => root.unmount());
    container.remove();
  }
});

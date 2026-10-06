// @vitest-environment happy-dom
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { Button, ControlPlaneFrame } from "@colossus/ui";
import {
  ConversationEntry,
  ConversationTimeline,
  ConversationActivity,
} from "@colossus/ui/conversation";

type Item = { id: string; title: string; status: string; created: number };
const items: Item[] = Array.from({ length: 35 }, (_, index) => ({
  id: String(index),
  title: `Task ${index + 1}`,
  status: index % 2 ? "Finished" : "Active",
  created: index,
}));
const columns: DataTableColumn<Item>[] = [
  {
    id: "title",
    label: "Task",
    value: (row) => row.title,
    rowHeader: true,
    hideable: false,
  },
  {
    id: "status",
    label: "Status",
    value: (row) => row.status,
    filter: {
      label: "Filter status",
      options: [
        { value: "", label: "All statuses" },
        { value: "Active", label: "Active" },
      ],
      matches: (row, value) => !value || row.status === value,
    },
  },
  { id: "created", label: "Created", value: (row) => row.created },
];
let container: HTMLDivElement, root: Root;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});
async function render(data = items, key = "one", definitions = columns) {
  await act(() =>
    root.render(
      <DataTable
        key={key}
        data={data}
        columns={definitions}
        getRowId={(row) => row.id}
        label="Tasks"
        itemLabel="tasks"
        search={{ columnId: "title", label: "Search tasks" }}
        initialSorting={[{ id: "created", desc: true }]}
      />,
    ),
  );
}
function button(label: string) {
  const result = [
    ...document.querySelectorAll<HTMLButtonElement>("button"),
  ].find(
    (node) =>
      node.getAttribute("aria-label") === label ||
      node.textContent?.trim() === label,
  );
  if (!result) throw new Error(`Missing button: ${label}`);
  return result;
}
async function click(node: HTMLElement) {
  await act(() => {
    node.dispatchEvent(
      new PointerEvent("pointerdown", {
        bubbles: true,
        button: 0,
        pointerType: "mouse",
      }),
    );
    node.click();
  });
}
const titles = () =>
  [...container.querySelectorAll('tbody th[scope="row"]')].map(
    (node) => node.textContent,
  );
it("shared frame preserves timeline entries and the full navigation name behind a compact caption", async () => {
  await act(() =>
    root.render(
      <ControlPlaneFrame
        current="admin"
        navigation={[
          {
            id: "admin",
            label: "Administration",
            shortLabel: "Admin",
            icon: <span aria-hidden="true" />,
          },
        ]}
        onNavigate={() => {}}
      >
        <ConversationTimeline>
          <ConversationEntry role="user" content="Retained human turn" />
          <ConversationEntry
            role="assistant"
            content="**Retained assistant reply**"
          />
        </ConversationTimeline>
      </ControlPlaneFrame>,
    ),
  );
  expect(container.querySelectorAll(".shared-conversation-entry")).toHaveLength(
    2,
  );
  expect(
    container.querySelector(".shared-conversation-timeline")?.textContent,
  ).toContain("Retained human turn");
  expect(container.querySelector(".shared-markdown strong")?.textContent).toBe(
    "Retained assistant reply",
  );
  const navigation = container.querySelector(
    'button[aria-label="Administration"]',
  );
  expect(navigation?.textContent).toBe("Admin");
  expect(navigation?.getAttribute("title")).toBe("Administration");
});
const status = () => container.querySelector('[role="status"]')!.textContent;
it("bounds shared activity and assistant brand images before conversation CSS loads", async () => {
  await act(() =>
    root.render(
      <>
        <ConversationActivity
          description="1 action"
          statusLabel="Completed"
          tone="success"
        >
          <p>Released activity</p>
        </ConversationActivity>
        <ConversationEntry
          role="assistant"
          content="Retained reply"
          markdown={false}
        />
      </>,
    ),
  );
  const images = [
    ...container.querySelectorAll<HTMLImageElement>(
      ".run-activity-mark img, .shared-assistant-marker img",
    ),
  ];
  expect(images).toHaveLength(2);
  for (const image of images) {
    expect(image.width).toBe(17);
    expect(image.height).toBe(17);
    expect(image.getAttribute("width")).toBe("17");
    expect(image.getAttribute("height")).toBe("17");
    expect(image.alt).toBe("");
  }
});
async function select(label: string, value: string) {
  await click(
    document.querySelector<HTMLElement>(
      `[role="combobox"][aria-label="${label}"]`,
    )!,
  );
  const option = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((node) => node.textContent?.trim() === value)!;
  await click(option);
}
async function search(value: string) {
  const input = container.querySelector<HTMLInputElement>(
    'input[type="search"]',
  )!;
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
describe("shared data table interactions", () => {
  it("sorts all loaded rows and exposes the sort direction", async () => {
    await render();
    expect(titles()[0]).toBe("Task 35");
    await click(button("Sort by Task"));
    expect(titles().slice(0, 3)).toEqual(["Task 1", "Task 2", "Task 3"]);
    expect(
      container.querySelector('thead th[aria-sort="ascending"]')?.textContent,
    ).toContain("Task");
    await click(button("Sort by Task"));
    expect(titles()[0]).toBe("Task 35");
  });
  it("keeps the current page through polling and resets it for searches", async () => {
    await render();
    await click(button("Next page"));
    expect(status()).toBe("11–20 of 35 tasks");
    await render(items.map((item) => ({ ...item })));
    expect(status()).toBe("11–20 of 35 tasks");
    await search("Task 35");
    expect(status()).toBe("1–1 of 1 tasks");
    expect(titles()).toEqual(["Task 35"]);
    expect(button("Previous page").disabled).toBe(true);
    await click(button("Clear filters"));
    expect(status()).toBe("1–10 of 35 tasks");
  });
  it("combines filters and leaves hidden columns searchable", async () => {
    await render();
    await select("Filter status", "Active");
    expect(status()).toBe("1–10 of 18 tasks");
    await click(button("Columns"));
    const checkbox = [
      ...document.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]'),
    ].find((node) => node.textContent?.trim() === "Status")!;
    await click(checkbox);
    expect(checkbox.getAttribute("aria-checked")).toBe("false");
    expect(container.querySelectorAll("thead th")).toHaveLength(2);
    expect(status()).toBe("1–10 of 18 tasks");
    await act(() =>
      document.activeElement?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      ),
    );
    await search("Task 35");
    expect(titles()).toEqual(["Task 35"]);
    await select("Filter status", "All statuses");
    expect(titles()).toEqual(["Task 35"]);
  });
  it("changes page size and clamps pages when live rows disappear", async () => {
    await render();
    await click(button("Next page"));
    await click(button("Next page"));
    await click(button("Next page"));
    expect(status()).toBe("31–35 of 35 tasks");
    await render(items.slice(0, 12));
    expect(status()).toBe("11–12 of 12 tasks");
    await select("Rows per page", "25");
    expect(status()).toBe("1–12 of 12 tasks");
    expect(button("Next page").disabled).toBe(true);
  });
  it("preserves focused row actions when live polling refreshes data and cell callbacks", async () => {
    const liveColumns = () =>
      columns.map((column) =>
        column.id === "title"
          ? { ...column, cell: (row: Item) => <button>{row.title}</button> }
          : { ...column },
      );
    await render(items, "one", liveColumns());
    const action = button("Task 35");
    action.focus();
    await render(
      items.map((item) => ({ ...item })),
      "one",
      liveColumns(),
    );
    expect(action.isConnected).toBe(true);
    expect(document.activeElement).toBe(action);
  });
  it("resets table preferences when switching projects and handles empty results", async () => {
    await render();
    await click(button("Next page"));
    await search("not present");
    expect(status()).toBe("0–0 of 0 tasks");
    expect(container.textContent).toContain("No results.");
    await render(items, "another-project");
    expect(status()).toBe("1–10 of 35 tasks");
    expect(
      container.querySelector<HTMLInputElement>('input[type="search"]')!.value,
    ).toBe("");
  });
});
it("shared button preserves native button and explicit form submit behavior", async () => {
  async function show(children: ReactNode) {
    await act(() => root.render(<form>{children}</form>));
  }
  await show(
    <>
      <Button>Cancel</Button>
      <Button type="submit">Save</Button>
    </>,
  );
  expect(button("Cancel").type).toBe("button");
  expect(button("Save").type).toBe("submit");
});

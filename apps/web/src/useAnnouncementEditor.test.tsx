// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Announcement, AnnouncementContent } from "./announcements";
import { useAnnouncementEditor } from "./useAnnouncementEditor";

const content: AnnouncementContent = {
  title: "New tools",
  summary: "Tools are now available.",
  body: "**Released feature**\n\nA Markdown description.",
  tone: "success",
  banner: true,
  priority: 75,
  expires_at: null,
};
const announcement = (id: string, revision = 1): Announcement => ({
  ...content,
  id,
  revision,
  publication_revision: revision,
  status: "published",
  published_at: "2026-10-22T12:00:00Z",
  created_at: "2026-10-22T12:00:00Z",
  updated_at: "2026-10-22T12:00:00Z",
});
interface Pending {
  path: string;
  options: RequestInit;
  respond: (body: unknown, status?: number) => void;
  reject: (reason: unknown) => void;
}
let container: HTMLDivElement, root: Root;
let current: ReturnType<typeof useAnnouncementEditor>;
let calls: Pending[];
const onSaved = vi.fn<(value: Announcement) => void>();
function Probe({ initial }: { initial: Announcement | null }) {
  current = useAnnouncementEditor(initial, onSaved);
  return <span>{current.error}</span>;
}
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  calls = [];
  onSaved.mockReset();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  vi.stubGlobal(
    "fetch",
    vi.fn(
      (path: string, options: RequestInit) =>
        new Promise<Response>((resolve, reject) => {
          calls.push({
            path,
            options,
            respond: (body, status = 200) =>
              resolve(new Response(JSON.stringify(body), { status })),
            reject,
          });
        }),
    ),
  );
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
async function render(initial: Announcement | null = null, key = "editor") {
  await act(() => root.render(<Probe key={key} initial={initial} />));
}
async function prepare() {
  await render();
  await act(() => current.setContent({ ...content }));
}
async function save() {
  await act(() => current.save("publish"));
}

it.each(["lost response", "server failure"])(
  "retains one exact publication proposal after %s and retries only on explicit activation",
  async (failure) => {
    await prepare();
    const stableId = current.id;
    await save();
    const first = calls[0]!;
    const packet = String(first.options.body);
    expect(first.path).toBe("/api/admin/announcements");
    expect(JSON.parse(packet)).toEqual({
      ...content,
      id: stableId,
      action: "publish",
    });
    await act(() =>
      failure === "lost response"
        ? first.reject(new TypeError("Connection lost"))
        : first.respond({ error: "storage" }, 503),
    );
    expect(current.locked).toBe(true);
    expect(current.uncertain).toMatchObject({
      id: stableId,
      content,
      action: "publish",
    });
    expect(onSaved).not.toHaveBeenCalled();
    await act(() =>
      current.setContent({ ...content, title: "An unrelated later edit" }),
    );
    await save();
    expect(calls).toHaveLength(1);
    await act(() => current.retry());
    expect(calls).toHaveLength(2);
    expect(calls[1]!.path).toBe(first.path);
    expect(calls[1]!.options.method).toBe("POST");
    expect(calls[1]!.options.body).toBe(packet);
    expect(current.id).toBe(stableId);
    await act(() =>
      calls[1]!.respond({ announcement: announcement(stableId) }),
    );
    expect(onSaved).toHaveBeenCalledExactlyOnceWith(announcement(stableId));
  },
);

it("reconciles an acknowledged publication with GET without issuing another POST", async () => {
  await prepare();
  await save();
  const stableId = current.id;
  await act(() => calls[0]!.reject(new TypeError("Response unavailable")));
  await act(() => {
    void current.checkStatus();
  });
  expect(calls[1]!.path).toBe(`/api/admin/announcements/${stableId}`);
  expect(calls[1]!.options.method).toBe("GET");
  const saved = announcement(stableId);
  await act(() => calls[1]!.respond({ announcement: saved }));
  expect(onSaved).toHaveBeenCalledExactlyOnceWith(saved);
  expect(calls.filter((call) => call.options.method === "POST")).toHaveLength(
    1,
  );
});

it.each(["different content", "not found"])(
  "keeps the uncertain proposal when a status check reports %s",
  async (result) => {
    await prepare();
    await save();
    const packet = calls[0]!.options.body;
    await act(() => calls[0]!.reject(new TypeError("Response unavailable")));
    await act(() => {
      void current.checkStatus();
    });
    await act(() =>
      result === "not found"
        ? calls[1]!.respond({ error: "not_found" }, 404)
        : calls[1]!.respond({
            announcement: {
              ...announcement(current.id),
              body: "A different administrator's content",
            },
          }),
    );
    expect(onSaved).not.toHaveBeenCalled();
    expect(current.locked).toBe(true);
    expect(current.uncertain?.content).toEqual(content);
    expect(current.error).toContain(
      result === "not found" ? "not available yet" : "differs",
    );
    await save();
    expect(calls).toHaveLength(2);
    await act(() => current.retry());
    expect(calls[2]!.options.body).toBe(packet);
  },
);

it("requires reloading after a confirmed edit conflict rather than retrying a stale revision", async () => {
  const original = announcement("existing", 7);
  await render(original);
  await act(() => current.setContent({ ...content, title: "Reviewed change" }));
  await save();
  expect(calls[0]!.path).toBe("/api/admin/announcements/existing");
  expect(calls[0]!.options.method).toBe("PATCH");
  expect(JSON.parse(String(calls[0]!.options.body))).toMatchObject({
    revision: 7,
    title: "Reviewed change",
    action: "publish",
  });
  await act(() => calls[0]!.respond({ error: "conflict" }, 409));
  expect(current.conflict).toBe(true);
  expect(current.locked).toBe(true);
  expect(current.uncertain).toBeNull();
  expect(current.error).toContain("Reload its current version");
  await save();
  await act(() => current.retry());
  expect(calls).toHaveLength(1);
  expect(onSaved).not.toHaveBeenCalled();
  await render(announcement("existing", 8), "reloaded-editor");
  expect(current.conflict).toBe(false);
  await save();
  expect(JSON.parse(String(calls[1]!.options.body))).toMatchObject({
    revision: 8,
  });
});

it("aborts a disposed editor and fences its delayed successful response", async () => {
  await prepare();
  await save();
  const oldId = current.id;
  const pending = calls[0]!;
  await render(null, "new-editor");
  expect((pending.options.signal as AbortSignal).aborted).toBe(true);
  expect(current.id).not.toBe(oldId);
  await act(() => pending.respond({ announcement: announcement(oldId) }));
  expect(onSaved).not.toHaveBeenCalled();
  expect(current.uncertain).toBeNull();
  expect(current.error).toBe("");
});

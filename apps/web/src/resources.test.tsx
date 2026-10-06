// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useResource } from "./resources";
it("discards a deferred prior-account response at the same resource URL", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const element = document.createElement("div");
  document.body.append(element);
  const root = createRoot(element);
  const pending: {
    resolve: (response: Response) => void;
    signal: AbortSignal;
  }[] = [];
  vi.stubGlobal(
    "fetch",
    vi
      .fn()
      .mockImplementation(
        (_path: string, options: { signal: AbortSignal }) =>
          new Promise<Response>((resolve) =>
            pending.push({ resolve, signal: options.signal }),
          ),
      ),
  );
  function View({ identity }: { identity: string }) {
    const { data } = useResource<{ label: string }>(
      "/api/dashboard",
      0,
      identity,
    );
    return <span>{data?.label ?? "Loading"}</span>;
  }
  try {
    await act(() => root.render(<View identity="user-a" />));
    await act(() => root.render(<View identity="user-b" />));
    expect(pending[0]!.signal.aborted).toBe(true);
    await act(async () =>
      pending[0]!.resolve(
        new Response(JSON.stringify({ label: "Prior account private data" })),
      ),
    );
    expect(element.textContent).toBe("Loading");
    await act(async () =>
      pending[1]!.resolve(
        new Response(JSON.stringify({ label: "Current account" })),
      ),
    );
    expect(element.textContent).toBe("Current account");
  } finally {
    await act(() => root.unmount());
    element.remove();
    vi.unstubAllGlobals();
  }
});

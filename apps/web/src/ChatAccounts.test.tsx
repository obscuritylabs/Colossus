// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatAccounts } from "./ChatAccounts";
let root: Root, container: HTMLDivElement;
const json = (data: unknown) => new Response(JSON.stringify(data));
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function render(project = "p") {
  await act(() =>
    root.render(<ChatAccounts project={project} identity="alice" canConnect />),
  );
}
it("waits for project availability before reading account connections", async () => {
  const paths: string[] = [];
  let resolveAccounts!: (value: Response) => void;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string) => {
      paths.push(path);
      return new Promise<Response>((resolve) => {
        resolveAccounts = resolve;
      });
    }),
  );
  await render();
  expect(paths).toEqual(["/api/projects/p/chat/accounts"]);
  await act(() =>
    resolveAccounts(
      json({
        accounts: [],
        execution: {
          enabled: false,
          settings_editable: false,
          reason: "Deployment operator setup required.",
        },
      }),
    ),
  );
  expect(paths).toHaveLength(1);
  expect(container.textContent).toContain(
    "Deployment operator setup required.",
  );
  expect(container.querySelector('[role="alert"]')).toBeNull();
});
it("sends one OAuth request and ignores its completion after an A to B to A scope change", async () => {
  const writes: string[] = [];
  let complete!: (value: Response) => void;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (path: string, options: RequestInit) => {
      if (options.method !== "GET") {
        writes.push(path);
        return new Promise<Response>((resolve) => {
          complete = resolve;
        });
      }
      return json(
        path.endsWith("/accounts")
          ? { accounts: [] }
          : {
              connections: [
                {
                  id: "docs",
                  label: "Docs",
                  oauth: true,
                  authentication: "personal",
                  authenticated: false,
                },
              ],
            },
      );
    }),
  );
  await render();
  const connect = [...container.querySelectorAll("button")].find(
    (button) => button.textContent === "Connect",
  )!;
  await act(() => {
    connect.click();
    connect.click();
  });
  expect(writes).toEqual(["/api/projects/p/chat/connections/docs/auth"]);
  await render("q");
  await render("p");
  await act(() =>
    complete(json({ authorization_url: "https://auth.example/late" })),
  );
  expect(
    container.querySelector('a[href="https://auth.example/late"]'),
  ).toBeNull();
  expect(container.textContent).not.toContain("Working…");
});

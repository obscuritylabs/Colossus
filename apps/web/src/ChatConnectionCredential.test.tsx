// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChatConnectionCredential } from "./ChatConnectionCredential";
let root: Root, container: HTMLDivElement;
const onSaved = vi.fn(),
  onError = vi.fn();
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  localStorage.clear();
  sessionStorage.clear();
  onSaved.mockReset();
  onError.mockReset();
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response("{}")),
  );
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function render(
  binding = "reviewed-binding",
  endpoint = "https://api.example/v1",
  disabled = false,
) {
  await act(() =>
    root.render(
      <ChatConnectionCredential
        path="/api/projects/p/chat/providers/model/credential"
        binding={binding}
        endpoint={endpoint}
        disabled={disabled}
        onSaved={onSaved}
        onError={onError}
      />,
    ),
  );
}
it("writes only to the reviewed credential binding and clears the secret without persistence", async () => {
  await render();
  const input = container.querySelector("input")!;
  input.value = "synthetic-personal-key";
  await act(() =>
    container
      .querySelector("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  expect(input.value).toBe("");
  expect(fetch).toHaveBeenCalledWith(
    "/api/projects/p/chat/providers/model/credential",
    expect.objectContaining({
      body: JSON.stringify({
        secret: "synthetic-personal-key",
        credential_binding: "reviewed-binding",
      }),
    }),
  );
  expect(container.textContent).toContain("https://api.example/v1");
  expect(container.innerHTML).not.toContain("synthetic-personal-key");
  expect(localStorage.length).toBe(0);
  expect(sessionStorage.length).toBe(0);
  expect(onSaved).toHaveBeenCalledOnce();
});
it("discards a typed credential when its endpoint, binding, or authority changes", async () => {
  await render();
  container.querySelector("input")!.value = "old-destination-key";
  await render("new-binding", "https://changed.example/v1");
  expect(container.querySelector("input")!.value).toBe("");
  container.querySelector("input")!.value = "another-key";
  await render("new-binding", "https://changed.example/v1", true);
  expect(container.querySelector("input")!.value).toBe("");
  expect(container.querySelector("input")!.disabled).toBe(true);
  expect(fetch).not.toHaveBeenCalled();
});

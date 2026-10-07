// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  NavigationProvider,
  RouteLink,
  navigate,
  navigateBack,
  rememberSignInReturn,
  restoreSignInReturn,
  useNavigation,
} from "./navigation";
import { AgentSidebar } from "./AgentScope";
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState(null, "", "/");
  sessionStorage.clear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  history.replaceState(null, "", "/");
  vi.restoreAllMocks();
});
function View() {
  const navigation = useNavigation();
  return (
    <>
      <output>{navigation.href}</output>
      <RouteLink href="/fleet">Fleet</RouteLink>
      <RouteLink href="/projects/p/threads/t">Conversation</RouteLink>
    </>
  );
}
async function changed() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10));
  });
}
it("updates real URLs and restores the view through Back and Forward", async () => {
  await act(() =>
    root.render(
      <NavigationProvider>
        <View />
      </NavigationProvider>,
    ),
  );
  await act(() =>
    container.querySelector<HTMLAnchorElement>('a[href="/fleet"]')!.click(),
  );
  await act(() =>
    container
      .querySelector<HTMLAnchorElement>('a[href="/projects/p/threads/t"]')!
      .click(),
  );
  expect(location.pathname).toBe("/projects/p/threads/t");
  await act(() => navigateBack("/fleet"));
  await changed();
  expect(container.querySelector("output")!.textContent).toBe("/fleet");
  await act(() => history.forward());
  await changed();
  expect(container.querySelector("output")!.textContent).toBe(
    "/projects/p/threads/t",
  );
});
it("uses actual sidebar Back history without pushing another Fleet entry and falls back for a direct load", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async () =>
      new Response(JSON.stringify({ threads: [], next_cursor: null })),
  );
  navigate("/fleet?project=p");
  navigate("/projects/p/agents/n/overview");
  await act(() =>
    root.render(
      <NavigationProvider>
        <AgentSidebar
          project="p"
          nodes={[]}
          hosts={[]}
          agentId="n"
          selected=""
          onAgent={() => {}}
          onOpen={() => {}}
          onNew={() => {}}
          onBack={() => navigateBack("/fleet?project=p")}
        />
      </NavigationProvider>,
    ),
  );
  const link = container.querySelector<HTMLAnchorElement>(
    'a[href="/fleet?project=p"]',
  )!;
  expect(link.textContent?.trim()).toBe("Back");
  const back = vi.spyOn(history, "back"),
    push = vi.spyOn(history, "pushState");
  await act(() => link.click());
  await changed();
  expect(back).toHaveBeenCalledTimes(1);
  expect(push).not.toHaveBeenCalled();
  expect(location.pathname + location.search).toBe("/fleet?project=p");
  history.replaceState(null, "", "/projects/p/agents/n/overview");
  await act(() => link.click());
  expect(back).toHaveBeenCalledTimes(1);
  expect(location.pathname + location.search).toBe("/fleet?project=p");
});
it("keeps modified clicks native and supplies a parent for a direct-link Back", async () => {
  history.replaceState(null, "", "/projects/p/threads/t");
  await act(() =>
    root.render(
      <NavigationProvider>
        <View />
      </NavigationProvider>,
    ),
  );
  const push = vi.spyOn(history, "pushState");
  const event = new MouseEvent("click", {
    bubbles: true,
    cancelable: true,
    button: 0,
    ctrlKey: true,
  });
  container.querySelector('a[href="/fleet"]')!.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(false);
  expect(push).not.toHaveBeenCalled();
  // Happy DOM performs native modified-link navigation in the same synthetic
  // window. Reopen the direct link before checking the separate Back contract.
  history.replaceState(null, "", "/projects/p/threads/t");
  await act(() => navigateBack("/projects/p/agents/n/overview"));
  expect(location.pathname).toBe("/projects/p/agents/n/overview");
});
it("resumes a bounded local deep link after sign-in without external redirects", () => {
  history.replaceState(null, "", "/projects/p/threads/t");
  rememberSignInReturn();
  history.replaceState(null, "", "/");
  restoreSignInReturn();
  expect(location.pathname).toBe("/projects/p/threads/t");
  expect(sessionStorage.getItem("colossus:web:sign-in-return")).toBeNull();
  history.replaceState(null, "", "/");
  sessionStorage.setItem(
    "colossus:web:sign-in-return",
    JSON.stringify({
      path: "https://other.example/",
      expires: Date.now() + 1000,
    }),
  );
  restoreSignInReturn();
  expect(location.pathname).toBe("/");
  expect(() => navigate("https://other.example/fleet")).toThrow(
    "Invalid Control Plane destination",
  );
});

// @vitest-environment happy-dom
import { act, StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { CredentialEditor } from "@colossus/ui/components/CredentialEditor";

it("preserves secure-entry focus and restores the trigger in Strict Mode", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  function Fixture() {
    const [open, setOpen] = useState(false);
    return (
      <>
        <button disabled={open} onClick={() => setOpen(true)}>
          Add credential
        </button>
        {open ? (
          <CredentialEditor
            description="Secure credential entry"
            busy={false}
            onCancel={() => setOpen(false)}
          >
            <label>
              Credential
              <input type="password" />
            </label>
          </CredentialEditor>
        ) : null}
      </>
    );
  }
  try {
    await act(() =>
      root.render(
        <StrictMode>
          <Fixture />
        </StrictMode>,
      ),
    );
    const trigger = container.querySelector("button")!;
    trigger.focus();
    await act(() => trigger.click());
    const input = container.querySelector("input")!;
    expect(document.activeElement).toBe(input);
    await act(() =>
      input.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Escape",
          bubbles: true,
          cancelable: true,
        }),
      ),
    );
    expect(container.querySelector("input")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  } finally {
    await act(() => root.unmount());
    container.remove();
  }
});

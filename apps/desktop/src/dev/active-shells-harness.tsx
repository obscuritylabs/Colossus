import { createRoot } from "react-dom/client";
import { ActiveShells } from "../components/tools/ActiveShells";
import type { ShellSession } from "../shellSessions";

// Browser-only mount: never imported by the production application.
export function mount(session: ShellSession) {
  const host = document.createElement("div");
  host.style.cssText =
    "position:fixed;inset:0;z-index:9999;background:var(--surface);overflow:auto";
  document.body.append(host);
  const root = createRoot(host);
  root.render(
    <ActiveShells
      scope="test-target"
      sessions={[session]}
      error={null}
      refresh={() => {}}
    />,
  );
  return () => {
    root.unmount();
    host.remove();
  };
}

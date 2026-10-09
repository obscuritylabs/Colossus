import { createRoot } from "react-dom/client";
import { ActiveShells } from "../components/tools/ActiveShells";
import { useShellSessions, type ShellSession } from "../shellSessions";

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

function PollingHarness({ scope }: { scope: string | null }) {
  const shells = useShellSessions(scope, false);
  return (
    <output
      data-testid="shell-polling"
      data-scope={scope ?? ""}
      data-loading={shells.loading}
      data-error={shells.error ?? ""}
    >
      {shells.sessions.length}
    </output>
  );
}

export function mountPolling() {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  root.render(<PollingHarness scope={null} />);
  return {
    setScope(scope: string | null) {
      root.render(<PollingHarness scope={scope} />);
    },
    unmount() {
      root.unmount();
      host.remove();
    },
  };
}

function LiveHarness({ scope }: { scope: string }) {
  const shells = useShellSessions(scope, false);
  return <ActiveShells {...shells} scope={scope} />;
}

export function mountLive(scope: string) {
  const host = document.createElement("div");
  host.style.cssText =
    "position:fixed;inset:0;z-index:9999;background:var(--surface);overflow:auto";
  document.body.append(host);
  const root = createRoot(host);
  root.render(<LiveHarness scope={scope} />);
  return () => {
    root.unmount();
    host.remove();
  };
}

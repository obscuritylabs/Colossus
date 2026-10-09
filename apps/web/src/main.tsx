import { lazy, Suspense, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "@colossus/ui/styles/theme.css";
import "@colossus/ui/styles/shadcn.css";
import "@colossus/ui/styles/control-plane.css";
import "@colossus/ui/styles/composer.css";
import "@colossus/ui/styles/select.css";
import "@colossus/ui/styles/settings.css";
import "@colossus/ui/styles/controls.css";
import "@colossus/ui/styles/application.css";
import "@colossus/ui/styles/catalog.css";
import "@colossus/ui/styles/conversation.css";
import "@colossus/ui/styles/work-presentation.css";
import "@colossus/ui/styles/workspace-sidebar.css";
import "@colossus/ui/styles/work-welcome.css";
import "./style.css";
const root = createRoot(document.getElementById("root")!);
if (
  import.meta.env.DEV &&
  new URLSearchParams(location.search).get("fixture") === "workspace-management"
) {
  const Fixture = lazy(() => import("./dev/workspace-management"));
  root.render(
    <Suspense fallback={<p role="status">Loading development fixture…</p>}>
      <Fixture />
    </Suspense>,
  );
} else {
  root.render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

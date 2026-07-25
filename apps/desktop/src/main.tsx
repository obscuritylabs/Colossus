import { createRoot } from "react-dom/client";

import { AppErrorBoundary } from "./components/AppErrorBoundary";
import "./styles.css";

const root = document.getElementById("root");

if (root === null) {
  throw new Error("Desktop root element is missing");
}

const terminalSurface =
  new URLSearchParams(window.location.search).get("surface") === "terminal";

if (terminalSurface) {
  void import("./TerminalWindow").then(({ default: TerminalWindow }) => {
    createRoot(root).render(
      <AppErrorBoundary>
        <TerminalWindow />
      </AppErrorBoundary>,
    );
  });
} else {
  void import("./App").then(({ default: App }) => {
    createRoot(root).render(
      <AppErrorBoundary>
        <App />
      </AppErrorBoundary>,
    );
  });
}

import { createRoot } from "react-dom/client";

import { AppErrorBoundary } from "./components/AppErrorBoundary";
import {
  AppearanceProvider,
  initializeAppearance,
} from "./theme/AppearanceProvider";
import "./theme/theme.css";
import "./styles.css";

const root = document.getElementById("root");

if (root === null) {
  throw new Error("Desktop root element is missing");
}

const terminalSurface =
  new URLSearchParams(window.location.search).get("surface") === "terminal";
const initialAppearance = initializeAppearance();

if (
  import.meta.env.DEV &&
  new URLSearchParams(window.location.search).get("fixture") === "plugin-studio"
) {
  void import("./dev/plugin-studio").then(({ default: PluginStudio }) => {
    createRoot(root).render(
      <AppearanceProvider initialPreference={initialAppearance}>
        <PluginStudio />
      </AppearanceProvider>,
    );
  });
} else if (terminalSurface) {
  void import("./TerminalWindow").then(({ default: TerminalWindow }) => {
    createRoot(root).render(
      <AppearanceProvider initialPreference={initialAppearance}>
        <AppErrorBoundary>
          <TerminalWindow />
        </AppErrorBoundary>
      </AppearanceProvider>,
    );
  });
} else {
  void import("./App").then(({ default: App }) => {
    createRoot(root).render(
      <AppearanceProvider initialPreference={initialAppearance}>
        <AppErrorBoundary>
          <App />
        </AppErrorBoundary>
      </AppearanceProvider>,
    );
  });
}

import { lazy, Suspense } from "react";
import { DesktopStartup } from "./components/DesktopStartup";
import { createRoot } from "react-dom/client";
import { SetupPresentationProvider } from "./SetupPresentation";
import { DesktopPreferencesProvider } from "./DesktopPreferencesProvider";

import { AppErrorBoundary } from "./components/AppErrorBoundary";
import {
  AppearanceProvider,
  initializeAppearance,
} from "./theme/AppearanceProvider";
import "@colossus/ui/styles/theme.css";
import "@colossus/ui/styles/shadcn.css";
import "@colossus/ui/styles/conversation.css";
import "@colossus/ui/styles/workspace-sidebar.css";
import "@colossus/ui/styles/work-welcome.css";
import "./styles.css";
import "@colossus/ui/styles/composer.css";
import "@colossus/ui/styles/select.css";
import "@colossus/ui/styles/settings.css";

const root = document.getElementById("root");

if (root === null) {
  throw new Error("Desktop root element is missing");
}

const terminalSurface =
  new URLSearchParams(window.location.search).get("surface") === "terminal";
const initialAppearance = initializeAppearance();

if (
  import.meta.env.DEV &&
  new URLSearchParams(window.location.search).get("fixture") === "setup"
) {
  void import("./dev/provider-setup-studio").then(
    async ({ default: SetupStudio }) => {
      const { installSetupPreviewApi } =
        await import("./dev/setup-preview-api");
      installSetupPreviewApi();
      createRoot(root).render(
        <AppearanceProvider initialPreference={initialAppearance}>
          <SetupStudio
            configured={false}
            workspaceSelected={false}
            hasCredential={false}
            showControls={false}
          />
        </AppearanceProvider>,
      );
    },
  );
} else if (
  import.meta.env.DEV &&
  new URLSearchParams(window.location.search).get("fixture") === "cloud"
) {
  void import("./dev/cloud-connection-preview").then(
    ({ default: CloudPreview }) => {
      createRoot(root).render(
        <AppearanceProvider initialPreference={initialAppearance}>
          <CloudPreview />
        </AppearanceProvider>,
      );
    },
  );
} else if (
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
} else if (
  new URLSearchParams(window.location.search).get("surface") ===
  "command-approval"
) {
  void import("./CommandReviewWindow").then(
    ({ default: CommandReviewWindow }) => {
      createRoot(root).render(
        <AppearanceProvider initialPreference={initialAppearance}>
          <AppErrorBoundary>
            <CommandReviewWindow />
          </AppErrorBoundary>
        </AppearanceProvider>,
      );
    },
  );
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
  const App = lazy(() => import("./App"));
  createRoot(root).render(
    <AppearanceProvider initialPreference={initialAppearance}>
      <DesktopPreferencesProvider>
        <AppErrorBoundary>
          <Suspense fallback={<DesktopStartup />}>
            <SetupPresentationProvider>
              <App />
            </SetupPresentationProvider>
          </Suspense>
        </AppErrorBoundary>
      </DesktopPreferencesProvider>
    </AppearanceProvider>,
  );
}

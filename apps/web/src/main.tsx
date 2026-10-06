import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "@colossus/ui/styles/theme.css";
import "@colossus/ui/styles/shadcn.css";
import "@colossus/ui/styles/composer.css";
import "@colossus/ui/styles/select.css";
import "@colossus/ui/styles/settings.css";
import "@colossus/ui/styles/controls.css";
import "@colossus/ui/styles/application.css";
import "@colossus/ui/styles/catalog.css";
import "@colossus/ui/styles/conversation.css";
import "@colossus/ui/styles/workspace-sidebar.css";
import "@colossus/ui/styles/work-welcome.css";
import "./style.css";
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

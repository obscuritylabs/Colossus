import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "@colossus/ui/styles/theme.css";
import "@colossus/ui/styles/composer.css";
import "@colossus/ui/styles/select.css";
import "./style.css";
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

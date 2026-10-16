import { createRoot } from "react-dom/client";
import { App } from "./App";
import "@colossus/ui/styles/theme.css";
import "@colossus/ui/styles/shadcn.css";
import "@colossus/ui/styles/controls.css";
import "@colossus/ui/styles/select.css";
import "@colossus/ui/styles/composer.css";
import "@colossus/ui/styles/conversation.css";
import "./style.css";

const root = createRoot(document.getElementById("root")!);
if (
  import.meta.env.DEV &&
  new URLSearchParams(location.search).get("fixture") === "login"
) {
  const { previewClient } = await import("./preview");
  root.render(<App initialClient={previewClient} />);
} else root.render(<App />);

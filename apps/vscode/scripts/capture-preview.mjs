// Browser screenshots of the actual bundled renderers, with explicit sample data.
// This script never opens a worker connection or reads enrollment/credential files.
import { readFile, writeFile, mkdir, readdir, cp } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { chromium } from "@playwright/test";

const directory = resolve("artifacts/review");
await mkdir(directory, { recursive: true });
const { version } = JSON.parse(await readFile("package.json", "utf8"));
const mark = `data:image/svg+xml;base64,${(await readFile("dist/colossus-mark.svg")).toString("base64")}`;
let style = `${await readFile("dist/theme.css", "utf8")}\n${await readFile("dist/style.css", "utf8")}
${await readFile("dist/composer.css", "utf8")}
${await readFile("dist/select.css", "utf8")}`;
for (const name of await readdir("dist/icons")) {
  const uri = `data:image/svg+xml;base64,${(await readFile(`dist/icons/${name}`)).toString("base64")}`;
  style = style.replaceAll(`icons/${name}`, uri);
}
const createdAt = "2026-10-03T10:20:00.000Z";
const run = {
  id: "run-navigation",
  sessionId: "session-workspace",
  title: "Improve workspace navigation",
  role: "primary",
  mode: "plan",
  status: "completed",
  createdAt,
  updatedAt: "2026-10-03T10:21:12.000Z",
  startedAt: createdAt,
  finishedAt: "2026-10-03T10:21:12.000Z",
  sequence: "184",
  pendingInteractions: 0,
};
const plan = {
  id: "plan-workspace-navigation",
  sourceRunId: run.id,
  sessionId: run.sessionId,
  title: run.title,
  revision: "3",
  status: "draft",
  goalId: "",
};
const output = `## Workspace navigation plan

Give Colossus a dedicated workspace view while keeping the conversation visible.

1. Add **Sessions**, **Plans**, and **Runtime** navigation to the primary sidebar.
2. Open saved run and plan state in an editor tab.
3. Keep chat in the secondary sidebar and reuse the desktop composer controls.
4. Verify reconnect recovery, canonical plan revisions, and narrow layouts.

### Validation

- Restore a saved conversation after reconnecting.
- Inspect the exact plan revision and durable run state.
- Keep a draft while the current run is working.

\`\`\`text
Left: workspace data
Center: editor and inspection
Right: conversation
\`\`\`

The runtime remains responsible for models, tools, and approvals.`;
const capabilities = [
  [
    "agent_runs.create",
    true,
    "Start an agent run with the enrolled role and tool ceiling.",
  ],
  [
    "agent_runs.read",
    true,
    "Read caller-owned durable run state and released output.",
  ],
  ["sessions.activity", true, "Inspect policy-released session activity."],
  ["agent_runs.cancel", true, "Request cooperative run cancellation."],
  [
    "plans.continue",
    true,
    "Worker supports canonical plan continuation. Extension controls are not implemented yet.",
  ],
  [
    "plugins.discovery",
    false,
    "Plugin discovery is unavailable for this enrollment.",
  ],
].map(([name, enabled, detail]) => ({ name, enabled, detail }));
const work = {
  connected: true,
  connecting: false,
  workspace: "Colossus",
  version: "0.11.7",
  sessionId: run.sessionId,
  sessions: [
    {
      id: run.sessionId,
      title: run.title,
      status: "completed",
      updatedAt: run.updatedAt,
    },
    {
      id: "session-tests",
      title: "Check worker reconnection",
      status: "completed",
    },
    {
      id: "session-policy",
      title: "Review approval boundaries",
      status: "cancelled",
    },
  ],
  runs: [
    run,
    {
      ...run,
      id: "run-prior",
      title: "Inspect current extension",
      mode: "execute",
      sequence: "143",
      createdAt: "2026-10-03T10:10:00.000Z",
    },
  ],
  plans: [
    plan,
    {
      ...plan,
      id: "plan-tests",
      title: "Worker reconnection checks",
      revision: "1",
      status: "executed",
      sessionId: "session-tests",
    },
  ],
  capabilities,
  historyHasMore: true,
  historyLoading: false,
  inspectionLoading: false,
  messages: [
    {
      id: "user:run-navigation",
      role: "user",
      text: "Plan the workspace layout and let me inspect the saved states.",
    },
    {
      id: "assistant:run-navigation",
      role: "assistant",
      text: "I saved a plan for the workspace layout.\n\n- **Left:** sessions, plans, and runtime capabilities.\n- **Center:** files, settings, and state inspection.\n- **Right:** your conversation with Colossus.\n\nOpen **Plans** in the workspace view to inspect the saved revision and output.",
    },
  ],
  tools: [
    {
      id: "tool-1",
      name: "filesystem.search",
      state: "completed",
      summary: "Located the workspace and composer components.",
    },
    {
      id: "tool-2",
      name: "plan.create",
      state: "completed",
      summary: "Saved the canonical workspace plan.",
    },
  ],
  interactions: [],
  context: [],
  busy: false,
  watching: false,
  status: "Completed",
  error: "",
  mode: "plan",
};
const inspection = {
  run,
  plan,
  output,
  model: "primary",
  provider: "local-worker",
  observedAt: run.finishedAt,
  activities: [
    {
      id: "activity-3",
      title: "Plan saved",
      summary: "Saved workspace navigation plan at revision 3.",
      kind: "plan",
      lane: "agent",
      status: "completed",
      startedAt: run.updatedAt,
      completedAt: run.updatedAt,
      result: "Canonical plan saved. Revision: 3. State: draft.",
    },
    {
      id: "activity-2",
      title: "Search workspace components",
      summary: "Located the sidebar navigation and chat composer.",
      kind: "tool",
      lane: "tools",
      status: "completed",
      startedAt: createdAt,
      completedAt: run.updatedAt,
      result: "Released result: workspace and composer components located.",
    },
    {
      id: "activity-1",
      title: "Agent run",
      summary: "Prepared a plan for the extension layout.",
      kind: "run",
      lane: "agent",
      status: "completed",
      startedAt: createdAt,
      completedAt: run.finishedAt,
      result: "Plan run completed.",
    },
  ],
  activityState: "Projection is up to date.",
  activityHasMore: false,
};
const preferences = {
  palette: "editor",
  sendShortcut: "modEnter",
  defaultMode: "plan",
  showToolActivity: true,
};
const settings = {
  preferences,
  workspace: "Colossus",
  connected: true,
  connecting: false,
  busy: false,
  hasSavedConnection: true,
  version: "0.11.7",
  role: "primary",
  error: "",
};
function json(value) {
  return JSON.stringify(value).replaceAll("<", "\\u003c");
}
async function renderer(name, page, payload, light = false) {
  const css =
    page === "settings" ? "settings" : page === "webview" ? "" : "workspace";
  const styles =
    style +
    (page === "settings"
      ? await readFile("dist/settings-frame.css", "utf8")
      : "") +
    (css ? await readFile(`dist/${css}.css`, "utf8") : "");
  const code = (await readFile(`dist/${page}.js`, "utf8")).replaceAll(
    "</script",
    "<\\/script",
  );
  const stub = `const sample=${json(payload)};window.acquireVsCodeApi=()=>({getState:()=>undefined,setState:()=>{},postMessage:action=>{if(action.type==='ready')window.postMessage(sample,'*');if(action.type==='refreshInspection')window.postMessage(sample,'*');if(action.type==='setPreference'){sample.view.preferences[action.name]=action.value;window.postMessage(sample,'*')}}});`;
  await writeFile(
    `${directory}/${name}.html`,
    `<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Colossus ${version} · ${name} · sample data</title><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'nonce-preview'; script-src 'nonce-preview'; img-src data:; connect-src 'none'; base-uri 'none'; form-action 'none'"><style nonce="preview">${styles}</style></head><body class="${light ? "vscode-light" : "vscode-dark"}" data-colossus-mark="${mark}"><div id="app"></div><script nonce="preview">${stub}</script><script nonce="preview">${code}</script></body></html>`,
  );
}
await renderer("chat", "webview", { type: "state", view: work, preferences });
await renderer("chat-colossus", "webview", {
  type: "state",
  view: work,
  preferences: { ...preferences, palette: "colossus" },
});
await renderer("workspace", "explorer", {
  type: "state",
  view: work,
  preferences,
});
await renderer("inspector", "inspector", {
  type: "inspection",
  view: inspection,
  connected: true,
  loading: false,
  error: "",
});
await renderer("settings", "settings", { type: "settings", view: settings });
await renderer(
  "chat-light",
  "webview",
  { type: "state", view: work, preferences },
  true,
);
await renderer(
  "workspace-light",
  "explorer",
  { type: "state", view: work },
  true,
);
await renderer(
  "inspector-light",
  "inspector",
  {
    type: "inspection",
    view: inspection,
    connected: true,
    loading: false,
    error: "",
  },
  true,
);
const activeChat = {
  type: "state",
  preferences,
  view: {
    ...work,
    busy: true,
    watching: true,
    status: "Waiting for approval",
    mode: "execute",
    messages: [
      {
        id: "u1",
        role: "user",
        text: "Implement the workspace view and run the checks.",
      },
      {
        id: "a1",
        role: "assistant",
        text: "The workspace view is implemented. I’m ready to run the focused checks after your approval.",
      },
    ],
    interactions: [
      {
        id: "sample-approval",
        title: "Run the focused extension checks",
        kind: "approval",
        respondable: true,
      },
    ],
    tools: [
      {
        id: "t1",
        name: "shell.run",
        state: "waiting approval",
        summary: "Run extension checks in the workspace.",
      },
    ],
  },
};
await renderer("active-chat", "webview", activeChat);
const hackerPreferences = { ...preferences, palette: "hacker" };
await renderer("chat-hacker", "webview", {
  type: "state",
  view: work,
  preferences: hackerPreferences,
});
await renderer("workspace-hacker", "explorer", {
  type: "state",
  view: work,
  preferences: hackerPreferences,
});
await renderer("inspector-hacker", "inspector", {
  type: "inspection",
  view: inspection,
  connected: true,
  loading: false,
  error: "",
  preferences: hackerPreferences,
});
await renderer("settings-hacker", "settings", {
  type: "settings",
  view: { ...settings, preferences: hackerPreferences },
});
await renderer("active-chat-hacker", "webview", {
  ...activeChat,
  preferences: hackerPreferences,
});
async function combined(light = false, palette = "editor") {
  const suffix = palette === "hacker" ? "-hacker" : light ? "-light" : "";
  await writeFile(
    `${directory}/layout${suffix}.html`,
    `<!doctype html><html lang="en"><head><meta charset="UTF-8"><title>Colossus ${version} layout · browser renderer preview</title><style>html,body{margin:0;height:100%;font:13px system-ui;background:${light ? "#eef3f8" : "#181818"};color:${light ? "#25364a" : "#e8eff8"}}header{height:38px;display:flex;align-items:center;padding:0 16px;border-bottom:1px solid ${light ? "#d4dee9" : "#363636"}}main{height:calc(100% - 39px);display:grid;grid-template-columns:290px minmax(0,1fr) 390px;gap:1px;background:${light ? "#d4dee9" : "#363636"}}section{min-width:0;display:flex;flex-direction:column}h2{height:28px;margin:0;padding:0 12px;display:flex;align-items:center;font-size:11px;font-weight:500;background:${light ? "#edf2f7" : "#181818"}}iframe{width:100%;flex:1;border:0}</style></head><body><header>Colossus ${version} · Actual extension renderers · Sample data · Browser preview</header><main><section><h2>LEFT · COLOSSUS WORKSPACE</h2><iframe title="Workspace data" src="workspace${suffix}.html"></iframe></section><section><h2>EDITOR · STATE INSPECTOR</h2><iframe title="State inspector" src="inspector${suffix}.html"></iframe></section><section><h2>RIGHT · COLOSSUS CHAT</h2><iframe title="Chat" src="chat${suffix}.html"></iframe></section></main></body></html>`,
  );
}
await combined();
await combined(true);
await combined(false, "hacker");
const browser = await chromium.launch({ headless: true });
const shots = [];
const errors = [];
async function shot(name, title, file, width, height, prepare) {
  const page = await browser.newPage({ viewport: { width, height } });
  page.on("pageerror", (error) => errors.push(`${name}: ${error.message}`));
  await page.goto(pathToFileURL(`${directory}/${file}.html`).href);
  await page
    .locator(file.startsWith("layout") ? "iframe" : "#app")
    .first()
    .waitFor();
  if (prepare) await prepare(page);
  await page.screenshot({ path: `${directory}/${name}.png`, fullPage: true });
  shots.push({ name, title });
  await page.close();
}
await shot(
  "01-full-layout",
  "Left workspace, center inspector, right chat",
  "layout",
  1600,
  1000,
);
await shot(
  "02-sessions",
  "Left sidebar · sessions and run states",
  "workspace",
  340,
  950,
);
await shot(
  "03-plans",
  "Left sidebar · saved plans and canonical revisions",
  "workspace",
  340,
  950,
  async (page) =>
    page.getByRole("button", { name: "Plans", exact: true }).click(),
);
await shot(
  "04-runtime",
  "Left sidebar · worker capabilities",
  "workspace",
  400,
  1100,
  async (page) =>
    page.getByRole("button", { name: "Runtime", exact: true }).click(),
);
await shot(
  "05-plan-state",
  "Editor · run and plan state",
  "inspector",
  1000,
  950,
);
await shot(
  "06-plan-output",
  "Editor · released saved plan output",
  "inspector",
  1000,
  950,
  async (page) =>
    page.getByRole("button", { name: "Output", exact: true }).click(),
);
await shot(
  "07-session-activity",
  "Editor · session activity and released results",
  "inspector",
  1000,
  950,
  async (page) => {
    await page
      .getByRole("button", { name: "Session activity", exact: true })
      .click();
    await page.getByText("Released result", { exact: true }).first().click();
  },
);
await shot(
  "08-chat",
  "Right chat · desktop tokens and composer",
  "chat",
  430,
  950,
);
await shot(
  "09-active-chat",
  "Right chat · approval, stop, and retained next-task draft",
  "active-chat",
  430,
  950,
  async (page) =>
    page
      .getByRole("textbox", { name: "Task for Colossus" })
      .fill("After the checks, inspect the saved plan revision."),
);
await shot(
  "10-settings-appearance",
  "Settings · Global appearance",
  "settings",
  1120,
  850,
);
await shot(
  "11-settings-defaults",
  "Settings · Global composer defaults",
  "settings",
  1120,
  850,
  async (page) =>
    page.getByRole("button", { name: "Defaults", exact: true }).click(),
);
await shot(
  "12-settings-connections",
  "Settings · Workspace connections",
  "settings",
  1120,
  850,
  async (page) => {
    await page.getByRole("button", { name: "Workspace", exact: true }).click();
  },
);
await shot(
  "13-settings-runtime",
  "Settings · Workspace runtime",
  "settings",
  1120,
  850,
  async (page) => {
    await page.getByRole("button", { name: "Workspace", exact: true }).click();
    await page.getByRole("button", { name: "Runtime", exact: true }).click();
  },
);
await shot(
  "14-settings-access",
  "Settings · Workspace access",
  "settings",
  1120,
  850,
  async (page) => {
    await page.getByRole("button", { name: "Workspace", exact: true }).click();
    await page.getByRole("button", { name: "Access", exact: true }).click();
  },
);
await shot(
  "15-light-layout",
  "Light theme · same left, editor, and right layout",
  "layout-light",
  1600,
  1000,
);
await shot(
  "16-narrow-chat",
  "Right chat · narrow 260 px layout",
  "chat",
  260,
  950,
);
await shot(
  "17-colossus-blue-chat",
  "Right chat · Colossus blue palette option",
  "chat-colossus",
  430,
  950,
);
await shot(
  "18-palette-picker",
  "Settings · shared accessible palette picker",
  "settings",
  1120,
  850,
  async (page) => {
    await page
      .getByRole("combobox", { name: "Surface palette", exact: true })
      .click();
  },
);
await shot(
  "19-compact-settings",
  "Settings · compact layout",
  "settings",
  360,
  850,
);
await shot(
  "23-hacker-layout",
  "Hacker · left workspace, state inspector, right chat",
  "layout-hacker",
  1600,
  1000,
);
await shot(
  "24-hacker-chat",
  "Hacker · conversation and composer",
  "chat-hacker",
  430,
  950,
);
await shot(
  "25-hacker-settings",
  "Hacker · shared settings and palette selection",
  "settings-hacker",
  1120,
  850,
);
await shot(
  "26-hacker-plans",
  "Hacker · left sidebar plans",
  "workspace-hacker",
  340,
  950,
  async (page) =>
    page.getByRole("button", { name: "Plans", exact: true }).click(),
);
await shot(
  "27-hacker-activity",
  "Hacker · session state and tool activity",
  "inspector-hacker",
  1000,
  950,
  async (page) => {
    await page
      .getByRole("button", { name: "Session activity", exact: true })
      .click();
    await page.getByText("Released result", { exact: true }).first().click();
  },
);
await shot(
  "28-hacker-approval",
  "Hacker · pending approval and retained draft",
  "active-chat-hacker",
  430,
  950,
  async (page) =>
    page
      .getByRole("textbox", { name: "Task for Colossus" })
      .fill("After the checks, inspect the saved plan revision."),
);
await shot(
  "29-hacker-compact",
  "Hacker · compact settings",
  "settings-hacker",
  360,
  850,
);
for (const [source, name, title] of [
  [
    "shared-ui-desktop-blue",
    "20-desktop-blue-settings",
    "Desktop · Colossus palette and shared settings frame",
  ],
  [
    "shared-ui-desktop-neutral-settings",
    "21-desktop-neutral-settings",
    "Desktop · optional neutral Dark+ palette",
  ],
  [
    "shared-ui-desktop-neutral-work",
    "22-desktop-neutral-work",
    "Desktop · work surface in neutral dark",
  ],
  [
    "shared-ui-desktop-hacker-settings",
    "30-desktop-hacker-settings",
    "Desktop · Hacker appearance settings",
  ],
  [
    "shared-ui-desktop-hacker-work",
    "31-desktop-hacker-work",
    "Desktop · Hacker work surface",
  ],
]) {
  await cp(
    `../desktop/output/playwright/${source}.png`,
    `${directory}/${name}.png`,
  );
  shots.push({ name, title });
}
await browser.close();
if (errors.length) throw new Error(errors.join("\n"));
const reviewShots = [...shots].sort((a, b) => {
  const aNumber = Number(a.name.slice(0, 2));
  const bNumber = Number(b.name.slice(0, 2));
  return Number(bNumber >= 23) - Number(aNumber >= 23) || aNumber - bNumber;
});
await writeFile(
  `${directory}/index.html`,
  `<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Colossus ${version} screenshot review</title><style>body{max-width:1400px;margin:auto;padding:28px;background:#07101d;color:#e8eff8;font:15px/1.6 system-ui}a{color:#6aa2ff}h1{margin:0}p{color:#91a2b8}nav{display:flex;gap:20px;flex-wrap:wrap;margin:20px 0}figure{margin:40px 0;border-top:1px solid #203149;padding-top:18px}figcaption{font-size:20px;margin-bottom:14px}img{display:block;max-width:100%;height:auto;border:1px solid #203149;border-radius:10px}section{display:grid;grid-template-columns:repeat(auto-fit,minmax(350px,1fr));gap:28px}section figure{margin:12px 0}section img{max-height:900px;width:auto}</style></head><body><h1>Colossus ${version} · UI review</h1><p>${shots.length} browser screenshots of the actual extension and Desktop renderers, with sample data. These are not native VS Code/Tauri captures or live worker results. Click any image for full resolution.</p><nav><a href="#23-hacker-layout">Hacker screenshots</a><a href="#30-desktop-hacker-settings">Desktop Hacker</a><a href="#01-full-layout">Other palettes</a><a href="layout.html">Open interactive renderer layout</a><a href="settings.html">Open settings renderer</a><a href="layout-light.html">Open light layout</a><a href="layout-hacker.html">Open Hacker layout</a><a href="settings-hacker.html">Open Hacker settings</a></nav>${reviewShots.map(({ name, title }) => `<figure id="${name}"><figcaption>${title}</figcaption><a href="${name}.png"><img src="${name}.png" alt="${title}" loading="lazy"></a></figure>`).join("")}</body></html>`,
);
process.stdout.write(
  `Captured ${shots.length} renderer screenshots: ${directory}/index.html\n`,
);

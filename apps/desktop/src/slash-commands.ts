import type { ApprovalMode, RunMode } from "./types";

export interface DesktopSlashCommandSuggestion {
  command: string;
  description: string;
  group: "Mode" | "Work" | "Navigation" | "Permissions";
}

export type DesktopSlashAction =
  | { type: "show_help" }
  | { type: "new_work" }
  | { type: "open_work_navigation" }
  | { type: "set_mode"; mode: RunMode; resetPlanRevision: boolean }
  | { type: "toggle_mode"; mode: "plan" | "research" }
  | { type: "show_mode_status"; mode: "plan" | "research" }
  | { type: "show_approval_mode" }
  | { type: "set_approval_mode"; mode: ApprovalMode }
  | {
      type: "select_surface";
      surface: "work" | "fleet" | "library" | "connections" | "settings";
    }
  | {
      type: "select_session_view";
      view: "conversation" | "plans";
    }
  | { type: "open_tui" };

export type DesktopSlashParseResult =
  | { type: "not_command" }
  | { type: "invalid"; message: string }
  | { type: "action"; action: DesktopSlashAction; canonical: string };

export const DESKTOP_SLASH_COMMANDS: readonly DesktopSlashCommandSuggestion[] =
  [
    {
      command: "/help",
      description: "Show supported Desktop commands",
      group: "Navigation",
    },
    {
      command: "/plan",
      description: "Toggle Plan mode",
      group: "Mode",
    },
    {
      command: "/plan new",
      description: "Start a new Plan draft",
      group: "Mode",
    },
    {
      command: "/plan on",
      description: "Enter Plan mode",
      group: "Mode",
    },
    {
      command: "/plan off",
      description: "Return to Execute mode",
      group: "Mode",
    },
    {
      command: "/plan status",
      description: "Show the current Plan mode",
      group: "Mode",
    },
    {
      command: "/plan list",
      description: "Open Plans for this task",
      group: "Work",
    },
    {
      command: "/plan show",
      description: "Open Plans for this task",
      group: "Work",
    },
    {
      command: "/plans",
      description: "Open Plans for this task",
      group: "Work",
    },
    {
      command: "/execute",
      description: "Enter Execute mode",
      group: "Mode",
    },
    {
      command: "/research",
      description: "Toggle Research mode",
      group: "Mode",
    },
    {
      command: "/research on",
      description: "Enter Research mode",
      group: "Mode",
    },
    {
      command: "/research off",
      description: "Return to Execute mode",
      group: "Mode",
    },
    {
      command: "/research status",
      description: "Show the current Research mode",
      group: "Mode",
    },
    {
      command: "/new",
      description: "Start new work",
      group: "Work",
    },
    {
      command: "/session new",
      description: "Start new work",
      group: "Work",
    },
    {
      command: "/resume",
      description: "Open task navigation",
      group: "Work",
    },
    {
      command: "/sessions",
      description: "Open task navigation",
      group: "Work",
    },
    {
      command: "/permissions",
      description: "Show the Desktop permission mode",
      group: "Permissions",
    },
    {
      command: "/permissions deny",
      description: "Deny approval obligations",
      group: "Permissions",
    },
    {
      command: "/permissions ask",
      description: "Ask before approval-required effects",
      group: "Permissions",
    },
    {
      command: "/permissions risk-auto",
      description: "Review eligible low-risk effects automatically",
      group: "Permissions",
    },
    {
      command: "/permissions full-access",
      description: "Satisfy approval obligations automatically",
      group: "Permissions",
    },
    {
      command: "/work",
      description: "Open the current conversation",
      group: "Navigation",
    },
    {
      command: "/agents",
      description: "Open Agents",
      group: "Navigation",
    },
    {
      command: "/artifacts",
      description: "Open the artifact library",
      group: "Navigation",
    },
    {
      command: "/connections",
      description: "Open Connections",
      group: "Navigation",
    },
    {
      command: "/settings",
      description: "Open Settings",
      group: "Navigation",
    },
    {
      command: "/tui",
      description: "Open the authenticated Colossus TUI",
      group: "Navigation",
    },
  ];

const ACTIONS = new Map<string, DesktopSlashAction>([
  ["/help", { type: "show_help" }],
  ["/plan", { type: "toggle_mode", mode: "plan" }],
  ["/plan new", { type: "set_mode", mode: "plan", resetPlanRevision: true }],
  ["/plan on", { type: "set_mode", mode: "plan", resetPlanRevision: false }],
  ["/plan off", { type: "set_mode", mode: "execute", resetPlanRevision: true }],
  ["/plan status", { type: "show_mode_status", mode: "plan" }],
  ["/plan list", { type: "select_session_view", view: "plans" }],
  ["/plan show", { type: "select_session_view", view: "plans" }],
  ["/plans", { type: "select_session_view", view: "plans" }],
  ["/execute", { type: "set_mode", mode: "execute", resetPlanRevision: true }],
  ["/research", { type: "toggle_mode", mode: "research" }],
  [
    "/research on",
    { type: "set_mode", mode: "research", resetPlanRevision: true },
  ],
  [
    "/research off",
    { type: "set_mode", mode: "execute", resetPlanRevision: true },
  ],
  ["/research status", { type: "show_mode_status", mode: "research" }],
  ["/new", { type: "new_work" }],
  ["/session new", { type: "new_work" }],
  ["/resume", { type: "open_work_navigation" }],
  ["/sessions", { type: "open_work_navigation" }],
  ["/permissions", { type: "show_approval_mode" }],
  ["/permissions deny", { type: "set_approval_mode", mode: "deny" }],
  ["/permissions ask", { type: "set_approval_mode", mode: "ask" }],
  ["/permissions risk-auto", { type: "set_approval_mode", mode: "risk_auto" }],
  [
    "/permissions full-access",
    { type: "set_approval_mode", mode: "full_access" },
  ],
  ["/work", { type: "select_session_view", view: "conversation" }],
  ["/agents", { type: "select_surface", surface: "fleet" }],
  ["/artifacts", { type: "select_surface", surface: "library" }],
  ["/connections", { type: "select_surface", surface: "connections" }],
  ["/settings", { type: "select_surface", surface: "settings" }],
  ["/tui", { type: "open_tui" }],
]);

function normalizedCommand(input: string): string {
  return input.trim().toLowerCase().split(/\s+/u).join(" ");
}

export function desktopSlashCommandSuggestions(
  input: string,
): readonly DesktopSlashCommandSuggestion[] {
  const query = input.trimStart().toLowerCase();
  if (!query.startsWith("/") || query.includes("\n") || query.includes("\r")) {
    return [];
  }
  return DESKTOP_SLASH_COMMANDS.filter(({ command }) =>
    command.startsWith(query),
  );
}

export function parseDesktopSlashCommand(
  input: string,
): DesktopSlashParseResult {
  const command = normalizedCommand(input);
  if (!command.startsWith("/")) {
    return { type: "not_command" };
  }
  const action = ACTIONS.get(command);
  if (action !== undefined) {
    return { type: "action", action, canonical: command };
  }
  if (command.startsWith("/plan ")) {
    return {
      type: "invalid",
      message:
        "That Plan command is not available in Desktop. Use /plans and the authenticated Plan cards for approval, revision, and execution, or open /tui for the full Plan command set.",
    };
  }
  if (command.startsWith("/research ")) {
    return {
      type: "invalid",
      message:
        "Usage: /research [on|off|status]. Submit a research question after switching modes.",
    };
  }
  if (command.startsWith("/permissions ")) {
    return {
      type: "invalid",
      message: "Usage: /permissions [deny|ask|risk-auto|full-access].",
    };
  }
  return {
    type: "invalid",
    message: `Unknown Desktop command “${command}”. Type /help to see supported commands.`,
  };
}

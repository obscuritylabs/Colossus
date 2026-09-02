import { describe, expect, it } from "vitest";

import {
  DESKTOP_SLASH_COMMANDS,
  desktopSlashCommandSuggestions,
  parseDesktopSlashCommand,
} from "./slash-commands";

describe("Desktop slash commands", () => {
  it("parses the core Plan mode commands as local UI actions", () => {
    expect(parseDesktopSlashCommand("/plan")).toEqual({
      type: "action",
      canonical: "/plan",
      action: { type: "toggle_mode", mode: "plan" },
    });
    expect(parseDesktopSlashCommand("  /PLAN   NEW  ")).toEqual({
      type: "action",
      canonical: "/plan new",
      action: {
        type: "set_mode",
        mode: "plan",
        resetPlanRevision: true,
      },
    });
    expect(parseDesktopSlashCommand("/plan off")).toMatchObject({
      type: "action",
      action: { type: "set_mode", mode: "execute" },
    });
  });

  it("keeps unknown and TUI-only Plan commands out of model prompts", () => {
    expect(parseDesktopSlashCommand("/plan execute goal 5")).toEqual({
      type: "invalid",
      message:
        "That Plan command is not available in Desktop. Use /plans and the authenticated Plan cards for approval, revision, and execution, or open /tui for the full Plan command set.",
    });
    expect(parseDesktopSlashCommand("/does-not-exist")).toMatchObject({
      type: "invalid",
      message: expect.stringContaining("Unknown Desktop command"),
    });
    expect(parseDesktopSlashCommand("Please plan this change")).toEqual({
      type: "not_command",
    });
  });

  it("returns bounded prefix completions and rejects multiline command drafts", () => {
    expect(
      desktopSlashCommandSuggestions("/plan ").map(({ command }) => command),
    ).toEqual([
      "/plan new",
      "/plan on",
      "/plan off",
      "/plan status",
      "/plan list",
      "/plan show",
    ]);
    expect(desktopSlashCommandSuggestions("/plan\nDo the work")).toEqual([]);
    expect(desktopSlashCommandSuggestions("ordinary prompt")).toEqual([]);
  });

  it("keeps every advertised command unique and backed by a local action", () => {
    const commands = DESKTOP_SLASH_COMMANDS.map(({ command }) => command);
    expect(new Set(commands).size).toBe(commands.length);

    for (const command of commands) {
      expect(parseDesktopSlashCommand(command)).toMatchObject({
        type: "action",
        canonical: command,
      });
    }
  });
});

import { describe, expect, it } from "vitest";
import type { UserConfig } from "vite";

import config from "../vite.config";

const desktopConfig = config as UserConfig;

describe("development server security", () => {
  it("binds only to loopback and denies every native source path", () => {
    expect(desktopConfig.server?.host).toBe("127.0.0.1");
    expect(desktopConfig.server?.strictPort).toBe(true);
    expect(desktopConfig.server?.fs?.strict).toBe(true);
    expect(desktopConfig.server?.fs?.deny).toContain("**/src-tauri/**");
  });
});

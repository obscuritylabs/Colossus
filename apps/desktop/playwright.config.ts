import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/browser",
  fullyParallel: false,
  forbidOnly: process.env.CI === "true",
  retries: process.env.CI === "true" ? 1 : 0,
  workers: 1,
  reporter: process.env.CI === "true" ? "github" : "line",
  use: {
    baseURL: "http://127.0.0.1:1421",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    viewport: { width: 880, height: 640 },
  },
  webServer: {
    command: "npm run dev -- --port 1421",
    url: "http://127.0.0.1:1421/?fixture=operations-studio",
    reuseExistingServer: process.env.CI !== "true",
    timeout: 120_000,
  },
});

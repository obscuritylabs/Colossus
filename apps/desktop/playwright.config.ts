import { defineConfig } from "@playwright/test";

const port = Number(process.env.COLOSSUS_PLAYWRIGHT_PORT ?? "1421");

export default defineConfig({
  testDir: "./tests/browser",
  fullyParallel: false,
  forbidOnly: process.env.CI === "true",
  retries: process.env.CI === "true" ? 1 : 0,
  workers: 1,
  reporter: process.env.CI === "true" ? "github" : "line",
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    viewport: { width: 880, height: 640 },
  },
  webServer: {
    command: `npm run dev -- --port ${port}`,
    url: `http://127.0.0.1:${port}/?fixture=operations-studio`,
    reuseExistingServer: process.env.CI !== "true",
    timeout: 120_000,
  },
});

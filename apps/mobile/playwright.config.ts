import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/browser",
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:1422",
    viewport: { width: 390, height: 844 },
    ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
      ? {
          launchOptions: {
            executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH,
          },
        }
      : {}),
  },
  webServer: {
    command: "npm run dev -- --host 127.0.0.1 --port 1422 --strictPort",
    url: "http://127.0.0.1:1422",
    reuseExistingServer: !process.env.CI,
  },
});

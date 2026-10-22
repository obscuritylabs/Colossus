import { defineConfig } from "@playwright/test";

const browserName =
  process.env.PLAYWRIGHT_BROWSER_NAME === "webkit" ? "webkit" : "chromium";

export default defineConfig({
  testDir: "./tests/browser",
  testMatch: "**/*.browser.ts",
  fullyParallel: false,
  forbidOnly: process.env.CI === "true",
  workers: 1,
  outputDir: "output/playwright/results",
  use: {
    browserName,
    baseURL: "http://127.0.0.1:5191",
    viewport: { width: 390, height: 844 },
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    ...(browserName === "chromium" &&
    process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH
      ? {
          launchOptions: {
            executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH,
          },
        }
      : {}),
  },
  webServer: {
    command:
      process.env.PLAYWRIGHT_WEB_DEV === "true"
        ? "npm run dev -- --host 127.0.0.1 --port 5191 --strictPort"
        : "npm run build -- --outDir output/playwright/dist && npm exec -- vite preview --outDir output/playwright/dist --host 127.0.0.1 --port 5191 --strictPort",
    url: "http://127.0.0.1:5191",
    reuseExistingServer: false,
  },
});

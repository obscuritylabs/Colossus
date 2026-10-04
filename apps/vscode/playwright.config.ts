import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "test/browser", timeout: 30000, fullyParallel: true,
  use: { baseURL: "http://127.0.0.1:4312", viewport: { width: 360, height: 850 }, screenshot: "only-on-failure" },
  webServer: { command: "node scripts/preview.mjs", url: "http://127.0.0.1:4312", reuseExistingServer: false },
});

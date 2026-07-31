import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

import { xtermFrozenPrototypeCompatibility } from "./build/xterm-frozen-prototype";

export default defineConfig({
  plugins: [xtermFrozenPrototypeCompatibility(), react()],
  optimizeDeps: {
    // Dependency pre-bundling bypasses the compatibility transform above.
    exclude: ["@xterm/xterm"],
  },
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    headers: {
      "Cross-Origin-Opener-Policy": "same-origin",
      "X-Content-Type-Options": "nosniff",
    },
    watch: {
      ignored: ["**/src-tauri/**"],
    },
    fs: {
      strict: true,
      deny: [
        ".env",
        ".env.*",
        "*.{crt,pem,key,p12,pfx,cer,der}",
        ".npmrc",
        ".yarnrc.yml",
        "**/.git/**",
        "**/src-tauri/**",
      ],
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "build/**/*.test.ts"],
  },
});

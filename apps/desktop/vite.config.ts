import { readFileSync } from "node:fs";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

import { d3ColorFrozenPrototypeCompatibility } from "./build/d3-color-frozen-prototype";
import { xtermFrozenPrototypeCompatibility } from "./build/xterm-frozen-prototype";
import { shikiGrammarLiterals } from "./build/shiki-grammar-literals";

export default defineConfig({
  plugins: [
    {
      name: "shared-ui-attribution",
      generateBundle() {
        this.emitFile({
          type: "asset",
          fileName: "shadcn-LICENSE.txt",
          source: readFileSync(
            new URL("../ui/assets/shadcn-LICENSE.txt", import.meta.url),
            "utf8",
          ),
        });
      },
    },
    tailwindcss(),
    d3ColorFrozenPrototypeCompatibility(),
    xtermFrozenPrototypeCompatibility(),
    shikiGrammarLiterals(),
    react(),
  ],
  optimizeDeps: {
    include: [
      "react-markdown",
      "react-resizable-panels",
      "recharts",
      "use-sync-external-store/shim",
      "use-sync-external-store/shim/with-selector",
    ],
    // Dependency pre-bundling bypasses the compatibility transform above.
    exclude: [
      "@colossus/ui",
      "@xterm/xterm",
      "@xyflow/system",
      "d3-color",
      "d3-dispatch",
      "d3-drag",
      "d3-ease",
      "d3-interpolate",
      "d3-selection",
      "d3-timer",
      "d3-transition",
      "d3-zoom",
    ],
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
        "**/development-credentials/**",
        "**/.authority-key.env",
        "**/src-tauri/**",
      ],
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts", "build/**/*.test.ts"],
  },
});

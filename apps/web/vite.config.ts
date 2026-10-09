import { readFileSync } from "node:fs";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
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
    react(),
  ],
  optimizeDeps: {
    // Keep shared context providers and their direct component imports together.
    exclude: ["@colossus/ui"],
    include: [
      "@xyflow/react",
      "react-markdown",
      "recharts",
      "react-resizable-panels",
      "use-sync-external-store/shim",
      "use-sync-external-store/shim/with-selector",
    ],
  },
  server: {
    strictPort: true,
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
      ],
    },
    proxy: {
      "/api": "http://127.0.0.1:8090",
      "/auth": "http://127.0.0.1:8090",
      "/health": "http://127.0.0.1:8090",
      "/docs": "http://127.0.0.1:8081",
    },
  },
  build: { target: "es2022" },
});

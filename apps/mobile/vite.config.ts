import { readFileSync } from "node:fs";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    tailwindcss(),
    react(),
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
  ],
  clearScreen: false,
  server: {
    host: "0.0.0.0",
    port: 1422,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
    fs: {
      strict: true,
      deny: [
        ".env",
        ".env.*",
        "*.{crt,pem,key,p12,pfx,cer,der,p8,jks,keystore,mobileprovision}",
        ".npmrc",
        "**/.git/**",
        "**/src-tauri/**",
      ],
    },
  },
  build: { target: "es2022" },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});

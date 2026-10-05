import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
export default defineConfig({
  plugins: [react()],
  server: {
    strictPort: true,
    proxy: {
      "/api": "http://127.0.0.1:8090",
      "/auth": "http://127.0.0.1:8090",
      "/health": "http://127.0.0.1:8090",
    },
  },
  build: { target: "es2022" },
});

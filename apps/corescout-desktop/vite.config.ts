import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri serves the built files from disk, so the base has to be relative. An
// absolute base produces a window that renders nothing and reports no error.
export default defineConfig({
  plugins: [react()],
  base: "./",
  clearScreen: false,
  server: { port: 5183, strictPort: true },
  build: { target: "chrome110", outDir: "dist", emptyOutDir: true },
  test: { environment: "jsdom", globals: true },
});

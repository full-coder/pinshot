import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The overlay is the only window in M0. `clearScreen: false` keeps Rust errors
// visible when the dev server and `cargo` run in the same terminal.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: "dist", target: "safari15", emptyOutDir: true },
});

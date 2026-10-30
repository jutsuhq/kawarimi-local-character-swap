import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a Node global; @types/node is not installed
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(() => ({
  plugins: [react(), tailwindcss()],
  // Tauri prints Rust errors to the same terminal, and expects a fixed port. Not Tauri's default 1420: Windows
  // reserves that range for Hyper-V on many machines.
  clearScreen: false,
  server: {
    port: 5420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 5421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**", "**/.models/**"] },
  },
}));

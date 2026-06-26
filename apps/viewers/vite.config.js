import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [sveltekit(), tailwindcss()],
  clearScreen: false,
  server: {
    // Distinct port from the other apps (shell 1420, settings 1421,
    // harness 1423, terminal 1425, files 1427) so they can all run in dev.
    port: 1429,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1430 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
}));

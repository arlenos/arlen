import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import { tailwindcssForSvelte } from "../../dev/build/tailwind-svelte-styles.js";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [sveltekit(), tailwindcssForSvelte()],
  clearScreen: false,
  server: {
    // Distinct port from the other apps (shell 1420, settings 1421,
    // harness 1423, terminal 1425) so they can all run in dev.
    port: 1427,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1527,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));

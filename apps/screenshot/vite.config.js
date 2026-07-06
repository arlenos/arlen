import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [sveltekit(), tailwindcss()],
  // `@arlen/tauri-plugin-menu` is a file: dependency, so a bundler resolves its
  // imports from the plugin's own location (sdk/), not this app's node_modules.
  // The plugin declares `@tauri-apps/api` as a peer dependency, which is only
  // installed here; dedupe forces resolution to this app's copy so the
  // production build does not fail to resolve `@tauri-apps/api/core`.
  resolve: { dedupe: ["@tauri-apps/api"] },
  clearScreen: false,
  server: {
    port: 1425,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1426,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));

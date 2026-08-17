import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import { withoutSvelteStyles } from "../../dev/build/tailwind-svelte-styles.js";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [sveltekit(), withoutSvelteStyles(tailwindcss())],
  resolve: { dedupe: ["@tauri-apps/api"] },
  clearScreen: false,
  server: {
    port: 1434,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1534,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));

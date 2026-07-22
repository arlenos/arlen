import { defineConfig, searchForWorkspaceRoot } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [sveltekit(), tailwindcss()],
  clearScreen: false,
  server: {
    // Distinct port from the Settings app (1421) so both can run in dev.
    port: 1423,
    strictPort: true,
    fs: {
      // The harness imports shared ui-kit components from `sdk/ui-kit/` (outside
      // this app dir), so the dev server must be allowed to serve the monorepo
      // root; without this Vite rejects those files ("outside serving allow list")
      // and the ui-kit sidebar/button/separator never render in dev.
      // @ts-expect-error process is a nodejs global
      allow: [searchForWorkspaceRoot(process.cwd()), "../../.."],
    },
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1424,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));

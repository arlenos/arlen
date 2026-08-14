// Tauri uses the static adapter (SPA mode); there is no Node SSR runtime.
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";
import { routesDir } from "../../dev/build/release-routes.js";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({
      fallback: "index.html",
    }),
    // Under ARLEN_RELEASE this is a routes directory without the `_`-prefixed
    // harnesses, so they leave the manifest, the chunks and the CSS together.
    // Unset - dev, check, screenshots - it is `src/routes` unchanged.
    files: { routes: routesDir(import.meta.dirname) },
    alias: {
      "@arlen/ui-kit": "../../sdk/ui-kit/src/lib",
      "@arlen/ui-kit/*": "../../sdk/ui-kit/src/lib/*",
    },
  },
};

export default config;

// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";
import { routesDir } from "../../dev/build/release-routes.js";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    // Under ARLEN_RELEASE this is a routes directory without the `_`-prefixed
    // harnesses, so they leave the manifest, the chunks and the CSS together.
    // Unset - dev, check, screenshots - it is `src/routes` unchanged.
    files: { routes: routesDir(import.meta.dirname) },
    adapter: adapter({
      fallback: "index.html",
    }),
    alias: {
      "@arlen/ui-kit": "../../sdk/ui-kit/src/lib",
      "@arlen/ui-kit/*": "../../sdk/ui-kit/src/lib/*",
    },
  },
};

export default config;

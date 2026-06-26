// Tauri uses the static adapter (SPA mode) because there is no Node SSR runtime.
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter({ fallback: "index.html" }),
    alias: {
      "@arlen/ui-kit": "../../sdk/ui-kit/src/lib",
      "@arlen/ui-kit/*": "../../sdk/ui-kit/src/lib/*",
    },
  },
};

export default config;

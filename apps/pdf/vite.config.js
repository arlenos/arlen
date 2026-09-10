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
    // WARM EVERY COMPONENT BEFORE THE BROWSER ASKS FOR ITS STYLES. Vite serves
    // each `<style>` block as its own module (`Foo.svelte?svelte&type=style`),
    // and when the browser requests one before the plugin has transformed
    // `Foo.svelte` itself, there is no compiled CSS to hand over and the RAW
    // `.svelte` SOURCE is injected as that component's stylesheet instead. The
    // browser then recovers at the first thing that parses as a rule, so the
    // sheet arrives unscoped and missing whatever came before that point.
    //
    // Measured on 11 September on the shell's main surface: 42 of its 63
    // component stylesheets were the raw source, deterministically, on a cold
    // server and a warm one. `files` and `settings` were clean, which is why it
    // went unseen - it only shows on a page that pulls in enough components at
    // once for the requests to outrun the transforms. Everything the harness
    // renders is a dev server, so this is what every screenshot and every axe
    // run of that surface had been measuring. `vite build` is unaffected.
    //
    // Warmup pre-transforms them at server start, so the cache is populated
    // before the first request. `render-wide.py` refuses a page that still has
    // one, because a fix that depends on winning a race needs somebody watching
    // it.
    warmup: { clientFiles: ["./src/**/*.svelte"] },
    port: 1452,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1552,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));

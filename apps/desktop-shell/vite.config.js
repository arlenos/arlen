import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

/**
 * Skip `@tailwindcss/vite` for Svelte-virtual style-block queries.
 *
 * `@tailwindcss/vite@4.x` matches every module id whose extension
 * hint is `.css` and tries to generate CSS for it via its
 * `generate:serve` plugin. SvelteKit emits a virtual module per
 * `<style>` block that looks like
 *   `.../routes/+page.svelte?svelte&type=style&lang.css`
 * Tailwind sees the `.css` hint, tries to read the underlying file
 * at the un-queried path, and gets the *raw* `.svelte` source
 * (script + template + style). Its CSS parser then chokes on the
 * first `<script>` token and either (a) serves an HTML error-
 * overlay blob as the "CSS" response, or (b) aborts the dev
 * server entirely — both symptoms we've hit.
 *
 * This plugin runs with `enforce: "pre"` so its `load` hook fires
 * before Tailwind's transform. For every id that matches the
 * Svelte virtual style-query pattern it returns an empty string,
 * which Tailwind passes through unchanged (no parse, no crash).
 * The real scoped-CSS content still comes from vite-plugin-svelte's
 * own pipeline — this plugin only intercepts the post-compile
 * query that Tailwind shouldn't be touching in the first place.
 *
 * Historical note: earlier we tried to "anchor" Tailwind's parser
 * by adding empty `<style>` blocks to every route file. That was
 * exactly backwards — those `<style>` blocks are what CAUSE the
 * virtual module to exist in the first place. Fixing the root
 * here lets us keep route files clean.
 */
/** @type {import("vite").Plugin} */
const skipTailwindForSvelteStyleQueries = {
  name: "lunaris:skip-tailwind-for-svelte-style-queries",
  enforce: "pre",
  load(/** @type {string} */ id) {
    if (
      id.includes("svelte") &&
      id.includes("type=style") &&
      id.includes("lang.css")
    ) {
      return "";
    }
    return null;
  },
};

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [sveltekit(), skipTailwindForSvelteStyleQueries, tailwindcss()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));

import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { fileURLToPath } from "node:url";

// Standalone vitest config (not the Vite/SvelteKit build): the unit tests are
// plain TypeScript over the app's pure logic. The `$lib` alias mirrors SvelteKit
// so a test can import from `$lib/...` (type-only imports are erased, real ones
// resolve here). jsdom for parity with the sibling apps and any future
// component-level test.
export default defineConfig({
  // The kit's i18n re-exports a .svelte helper, so the test transform needs
  // the Svelte plugin even though the suites themselves are plain TS.
  plugins: [svelte()],
  resolve: {
    alias: {
      $lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
      // The kit alias, mirrored from svelte.config.js: the menu test reads
      // labels through the app catalogue, which imports the kit's i18n.
      "@arlen/ui-kit": fileURLToPath(new URL("../../sdk/ui-kit/src/lib", import.meta.url)),
    },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
  },
});

import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

// Standalone vitest config (not the Vite/SvelteKit build): the unit tests are
// plain TypeScript. `jsdom` gives DOMPurify a DOM complete enough to actually
// sanitize against (happy-dom's is not, and DOMPurify silently no-ops there).
// The `$lib` alias mirrors SvelteKit so a test can value-import from `$lib/...`
// (type-only imports are erased and never needed resolution, but real ones do).
export default defineConfig({
  resolve: {
    alias: {
      $lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
    },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
  },
});

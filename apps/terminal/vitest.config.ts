import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

// Standalone vitest config (not the Vite/SvelteKit build): the unit tests are
// plain TypeScript over the app's pure logic. The `$lib` alias mirrors SvelteKit
// so a test can import from `$lib/...` (type-only imports are erased, real ones
// resolve here). jsdom for parity with the sibling apps and any future
// component-level test.
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

import { defineConfig } from "vitest/config";

// Standalone vitest config (not the Vite/SvelteKit build): the unit tests are
// plain TypeScript. `jsdom` gives DOMPurify a DOM complete enough to actually
// sanitize against (happy-dom's is not, and DOMPurify silently no-ops there).
export default defineConfig({
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
  },
});

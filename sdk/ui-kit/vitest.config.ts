import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vitest/config";

/// Test config for the kit. The pure-logic suites are environment-agnostic;
/// the a11y suite (`*.a11y.test.ts`) renders real components, so it needs a DOM
/// (jsdom) and the browser resolve condition for Svelte's client build. The
/// sveltekit plugin gives the same `$lib` resolution the components compile
/// against in the app.
export default defineConfig({
  plugins: [sveltekit()],
  resolve: {
    conditions: ["browser"],
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./vitest-setup.ts"],
  },
});

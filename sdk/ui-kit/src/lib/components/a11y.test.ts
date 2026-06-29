// @vitest-environment jsdom
/// The A11Y-R1 gate: render the kit primitives in their realistic, labelled
/// composition and assert axe-core finds zero violations. jsdom has no layout,
/// so axe checks roles / accessible names / ARIA (the static a11y layer);
/// contrast + visibility are A11Y-R2. Keyboard operation is covered by the
/// behavioural specs, not axe.
import { describe, it, expect } from "vitest";
import { render } from "@testing-library/svelte";
import axe from "axe-core";
import Kitchen from "./a11y-kitchen.svelte";

/// Run axe over a node and return a readable failure list.
async function violations(node: Element): Promise<string[]> {
  const results = await axe.run(node as HTMLElement, {
    resultTypes: ["violations"],
    // Contrast + visibility need real layout jsdom does not provide; they are
    // the A11Y-R2 (theme/contrast) strand, gated separately.
    rules: { "color-contrast": { enabled: false } },
  });
  return results.violations.map(
    (v) => `${v.id} (${v.impact}): ${v.help} [${v.nodes.length}x] -> ${v.nodes[0]?.target.join(" ")}`,
  );
}

describe("kit primitives: axe a11y gate", () => {
  it("the kitchen-sink fixture has zero axe violations", async () => {
    const { container } = render(Kitchen);
    const found = await violations(container);
    expect(found, `axe violations:\n${found.join("\n")}`).toEqual([]);
  });
});

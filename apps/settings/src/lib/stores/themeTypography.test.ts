/// The Appearance page must show the override that is actually in force.
///
/// Found by rendering on 17 August: with `size_base = "13px"` in the config and
/// the system rendering at 13, the Text size row read 14 - the theme's value.
/// `load()` read the resolved theme and never read the overrides beside it, so
/// the store started empty on every open and the row described a size the machine
/// was not using. This pins the read-back.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { readFileSync } from "node:fs";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { load, effective, overrides, isOverridden, TYPO_DEFAULTS } =
  await import("./themeTypography");
const { get } = await import("svelte/store");

/// The two commands the store reads, answered independently so a test can give a
/// theme one value and the config another - which is the whole point.
function backend(metrics: Record<string, string>, stored: unknown) {
  invoke.mockReset();
  invoke.mockImplementation((cmd: string) => {
    if (cmd === "theme_resolved_metrics") return Promise.resolve(metrics);
    if (cmd === "config_get") return Promise.resolve(stored);
    return Promise.reject(new Error("unexpected command " + cmd));
  });
}

beforeEach(() => overrides.set({}));

describe("load", () => {
  it("shows the override rather than the theme value when one is set", async () => {
    backend({ "typography.size_base": "14px" }, { size_base: "13px" });
    await load();
    expect(get(effective).sizeBase).toBe(13);
    expect(isOverridden(get(overrides), "sizeBase")).toBe(true);
  });

  it("shows the theme value when no override is stored", async () => {
    backend({ "typography.size_base": "14px" }, null);
    await load();
    expect(get(effective).sizeBase).toBe(14);
    expect(isOverridden(get(overrides), "sizeBase")).toBe(false);
  });

  it("keeps reading the theme when the overrides cannot be read", async () => {
    // A config that will not answer must not blank the page: the rows fall back
    // to the theme, which is what an unoverridden field looks like anyway.
    invoke.mockReset();
    invoke.mockImplementation((cmd: string) =>
      cmd === "theme_resolved_metrics"
        ? Promise.resolve({ "typography.size_base": "14px" })
        : Promise.reject(new Error("no config")),
    );
    await load();
    expect(get(effective).sizeBase).toBe(14);
  });

  it("carries the font families back too, not only the size", async () => {
    backend(
      { "typography.font_sans": "Inter Variable", "typography.font_mono": "JetBrains Mono" },
      { font_sans: "Cantarell" },
    );
    await load();
    expect(get(effective).fontSans).toBe("Cantarell");
    expect(get(effective).fontMono).toBe("JetBrains Mono");
  });
});

/// The floor under a failed read must be what the theme actually resolves.
///
/// Found on 15 September by reading the two side by side: `sizeBase` said 15
/// where `dark.toml` says `14px`, and `weightBold` said 700 where it says 600.
/// Nothing compared them, so the drift was invisible for as long as it took
/// somebody to open the page with no backend - the same shape as the terminal
/// swatch grid one store over, and the sound row before that.
describe("TYPO_DEFAULTS", () => {
  const THEME = new URL(
    "../../../../../sdk/theme/themes/dark.toml",
    import.meta.url,
  ).pathname;

  /// The `[typography]` table, key to its raw value, with quotes stripped.
  function themeTypography(): Record<string, string> {
    const text = readFileSync(THEME, "utf8");
    const block = text.match(/\[typography\]\n([\s\S]*?)(?=\n\[|$)/);
    if (!block) throw new Error("no [typography] table in " + THEME);
    const out: Record<string, string> = {};
    for (const line of block[1].split("\n")) {
      const m = line.match(/^(\w+)\s*=\s*(.+?)\s*$/);
      if (m) out[m[1]] = m[2].replace(/^"(.*)"$/, "$1");
    }
    return out;
  }

  it("carries the numbers the shipped theme carries", () => {
    const theme = themeTypography();
    // The store holds numbers; the theme holds a CSS length or a bare number.
    expect(TYPO_DEFAULTS.sizeBase).toBe(Number.parseFloat(theme.size_base));
    expect(TYPO_DEFAULTS.lineHeight).toBe(Number.parseFloat(theme.line_height));
    expect(TYPO_DEFAULTS.weightNormal).toBe(Number(theme.weight_normal));
    expect(TYPO_DEFAULTS.weightMedium).toBe(Number(theme.weight_medium));
    expect(TYPO_DEFAULTS.weightBold).toBe(Number(theme.weight_bold));
  });

  it("names the family the theme names first", () => {
    // The row is a picker over known families, so the floor holds the family
    // rather than the whole fallback stack the theme carries.
    const theme = themeTypography();
    const first = (stack: string) =>
      stack.split(",")[0].trim().replace(/\\?"/g, "");
    expect(first(theme.font_sans)).toBe(TYPO_DEFAULTS.fontSans);
    expect(first(theme.font_mono)).toContain("JetBrains");
  });
});

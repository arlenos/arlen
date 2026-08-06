import { describe, it, expect } from "vitest";
import { formatDecimal } from "./index";

describe("formatDecimal", () => {
  it("uses the reader's decimal mark", () => {
    // The failure it exists for: `toFixed` writes a point in every language.
    expect(formatDecimal(2.4, 1, "en")).toBe("2.4");
    expect(formatDecimal(2.4, 1, "de")).toBe("2,4");
  });

  it("groups large numbers the reader's way", () => {
    expect(formatDecimal(1234567, 0, "en")).toBe("1,234,567");
    expect(formatDecimal(1234567, 0, "de")).toBe("1.234.567");
  });

  it("keeps the digits asked for", () => {
    expect(formatDecimal(3, 0, "en")).toBe("3");
    expect(formatDecimal(3, 1, "en")).toBe("3.0");
  });
});

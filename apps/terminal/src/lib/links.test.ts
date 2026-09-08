import { describe, it, expect } from "vitest";
import { findLinks, logicalLine, cellAt, type BufferLike, type LineLike } from "./links";

describe("findLinks", () => {
  it("finds a bare URL in a line of output", () => {
    const [link] = findLinks("cloning from https://github.com/arlenos/arlen now");
    expect(link.url).toBe("https://github.com/arlenos/arlen");
    expect(link.start).toBe(13);
    expect(link.end).toBe(13 + link.url.length);
  });

  it("finds several on one line, in order", () => {
    const found = findLinks("see http://a.example/1 and https://b.example/2");
    expect(found.map((l) => l.url)).toEqual(["http://a.example/1", "https://b.example/2"]);
  });

  it("takes a mailto, because the opener does", () => {
    expect(findLinks("write to mailto:someone@example.com")[0].url).toBe(
      "mailto:someone@example.com",
    );
  });

  it("leaves alone a scheme the opener would refuse", () => {
    // `url.rs` accepts http, https and mailto and refuses the rest. Underlining
    // a `file://` path would offer a click that comes back as an error.
    expect(findLinks("file:///etc/passwd and ftp://example.com/x")).toEqual([]);
  });

  it("drops the full stop that ends the sentence", () => {
    expect(findLinks("read https://example.com/docs.")[0].url).toBe("https://example.com/docs");
  });

  it("drops a closing bracket the URL never opened", () => {
    expect(findLinks("(see https://example.com/x)")[0].url).toBe("https://example.com/x");
  });

  it("keeps a bracket that is part of the address", () => {
    expect(findLinks("https://en.wikipedia.org/wiki/Foo_(bar)")[0].url).toBe(
      "https://en.wikipedia.org/wiki/Foo_(bar)",
    );
  });

  it("unwinds punctuation and bracket together", () => {
    // The two rules compose: the stop has to go before the parenthesis is last.
    expect(findLinks("(https://example.com/x).")[0].url).toBe("https://example.com/x");
  });

  it("keeps a query string intact", () => {
    expect(findLinks("open https://example.com/s?q=1&r=2 please")[0].url).toBe(
      "https://example.com/s?q=1&r=2",
    );
  });

  it("refuses a scheme with nothing after it", () => {
    expect(findLinks("the https:// prefix")).toEqual([]);
  });

  it("finds nothing in ordinary output", () => {
    expect(findLinks("total 48\ndrwxr-xr-x 5 tim users 4096 Sep  8 12:00 src")).toEqual([]);
  });
});

/// A buffer of fixed-width rows, which is what xterm hands the provider.
function buffer(cols: number, rows: Array<{ text: string; wrapped?: boolean }>): BufferLike {
  const lines: LineLike[] = rows.map((r) => ({
    isWrapped: r.wrapped === true,
    translateToString: (trimRight?: boolean) =>
      trimRight === true ? r.text : r.text.padEnd(cols, " ").slice(0, cols),
  }));
  return { length: lines.length, getLine: (i) => lines[i] };
}

describe("logicalLine", () => {
  it("returns a single unwrapped row as itself", () => {
    const line = logicalLine(buffer(10, [{ text: "hello" }]), 0);
    expect(line).not.toBeNull();
    expect(line?.firstRow).toBe(0);
    expect(line?.cols).toBe(10);
    expect(line?.text).toBe("hello     ");
  });

  it("joins the rows a wrapped line occupies", () => {
    const b = buffer(8, [
      { text: "https://" },
      { text: "example.", wrapped: true },
      { text: "com/page", wrapped: true },
      { text: "next" },
    ]);
    const line = logicalLine(b, 1);
    expect(line?.firstRow).toBe(0);
    expect(line?.text).toBe("https://example.com/page");
  });

  it("stops at the row that starts a new line", () => {
    const b = buffer(4, [{ text: "abcd" }, { text: "efgh", wrapped: true }, { text: "ijkl" }]);
    expect(logicalLine(b, 0)?.text).toBe("abcdefgh");
  });

  it("has nothing to say about a row that is not there", () => {
    expect(logicalLine(buffer(4, [{ text: "abcd" }]), 7)).toBeNull();
  });
});

describe("a wrapped URL", () => {
  it("is found whole and points at the cells it starts and ends in", () => {
    const b = buffer(8, [
      { text: "https://" },
      { text: "example.", wrapped: true },
      { text: "com/x   ", wrapped: true },
    ]);
    const line = logicalLine(b, 2);
    expect(line).not.toBeNull();
    const [link] = findLinks(line!.text);
    expect(link.url).toBe("https://example.com/x");
    // xterm counts both axes from one: the first cell of the first row.
    expect(cellAt(line!, link.start)).toEqual({ x: 1, y: 1 });
    // The last character sits in the fifth column of the third row.
    expect(cellAt(line!, link.end - 1)).toEqual({ x: 5, y: 3 });
  });
});

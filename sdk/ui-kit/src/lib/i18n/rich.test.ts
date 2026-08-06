// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, it, expect } from "vitest";
import { mark, richParts } from "./rich";

/** Compact rendering of the parts, so a test asserts the shape rather than an
 *  object graph: text as-is, a mark as `[name]`. */
function shape(text: string): string {
  return richParts(text)
    .map((p) => (p.kind === "text" ? p.text : `[${p.name}]`))
    .join("");
}

describe("richParts", () => {
  it("splits a sentence around one marked term", () => {
    const formatted = `No displays reported. The compositor may not expose ${mark("tool")}.`;
    expect(shape(formatted)).toBe("No displays reported. The compositor may not expose [tool].");

    const parts = richParts(formatted);
    expect(parts).toHaveLength(3);
    expect(parts[1]).toEqual({ kind: "mark", name: "tool" });
  });

  it("follows the translation's word order rather than the argument order", () => {
    // The whole point of the exercise. `s.ext.install` names a command and a
    // directory; a language that puts the location first gets them in that order
    // out of the formatter, and each part still carries its own name, so the
    // render site wraps the right one in the right element.
    const en = `Run ${mark("cmd")} or drop a module into ${mark("dir")}.`;
    const de = `Legen Sie ein Modul in ${mark("dir")} ab oder führen Sie ${mark("cmd")} aus.`;

    expect(shape(en)).toBe("Run [cmd] or drop a module into [dir].");
    expect(shape(de)).toBe("Legen Sie ein Modul in [dir] ab oder führen Sie [cmd] aus.");

    const order = (s: string) =>
      richParts(s).flatMap((p) => (p.kind === "mark" ? [p.name] : []));
    expect(order(en)).toEqual(["cmd", "dir"]);
    expect(order(de)).toEqual(["dir", "cmd"]);
  });

  it("keeps a message with no marks in one piece", () => {
    expect(richParts("Nothing to style here.")).toEqual([
      { kind: "text", text: "Nothing to style here." },
    ]);
  });

  it("emits no empty text parts when a mark opens or closes the sentence", () => {
    // A leading or trailing empty string would render as an extra text node and,
    // in a flex row, as an extra gap.
    const parts = richParts(`${mark("term")} matched.`);
    expect(parts).toEqual([
      { kind: "mark", name: "term" },
      { kind: "text", text: " matched." },
    ]);
    expect(richParts(mark("only"))).toEqual([{ kind: "mark", name: "only" }]);
  });

  it("handles two marks with nothing between them", () => {
    expect(shape(`${mark("a")}${mark("b")}`)).toBe("[a][b]");
  });

  it("treats an unterminated mark as literal text and keeps the tail", () => {
    // A corrupt catalog must not swallow the rest of the sentence. Losing the tail
    // silently is worse than showing one odd character, because nobody reports the
    // half-sentence as a bug - it just reads oddly.
    const broken = `Active until \uE000expiry and more text`;
    const parts = richParts(broken);
    expect(parts).toEqual([{ kind: "text", text: broken }]);
    expect(parts.map((p) => (p.kind === "text" ? p.text : "")).join("")).toContain(
      "and more text",
    );
  });

  it("does not treat a user-supplied value as markup", () => {
    // The values interpolated here are filenames and search terms, which is exactly
    // the untrusted-ish text a positional split would trip over. A term containing
    // braces or angle brackets is just text.
    expect(shape(`No matches for "<b>{oops}</b>".`)).toBe(`No matches for "<b>{oops}</b>".`);
  });

  it("reads an empty mark name without hanging or dropping text", () => {
    expect(richParts(`a\uE000\uE001b`)).toEqual([
      { kind: "text", text: "a" },
      { kind: "mark", name: "" },
      { kind: "text", text: "b" },
    ]);
  });
});

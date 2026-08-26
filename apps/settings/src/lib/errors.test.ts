/// What the suppression predicate must catch, and what it must let through.
///
/// Both halves matter equally. Suppress too little and a person meets
/// `window.__TAURI_INTERNALS__` in the middle of a settings pane; suppress too
/// much and a backend's own useful words - "permission denied", "no space left
/// on device" - are swallowed and the surface says nothing they can act on.
import { describe, expect, it } from "vitest";
import { readsAsInternal } from "./errors";

describe("readsAsInternal", () => {
  it("suppresses the runtime errors a person cannot act on", () => {
    expect(
      readsAsInternal(
        "TypeError: undefined is not an object (evaluating 'window.__TAURI_INTERNALS__.invoke')",
      ),
    ).toBe(true);
    expect(readsAsInternal("ReferenceError: x is not defined")).toBe(true);
    expect(readsAsInternal("TypeError: foo.bar is not a function")).toBe(true);
  });

  it("shows a backend's own words, which are the useful kind", () => {
    expect(readsAsInternal("No space left on device (os error 28)")).toBe(false);
    expect(readsAsInternal("Permission denied (os error 13)")).toBe(false);
    expect(readsAsInternal("decode-image: unsupported JPEG progressive scan")).toBe(false);
    expect(readsAsInternal("")).toBe(false);
  });

  /// The input contract, pinned because it is the fragile part. WebKit is what
  /// Tauri renders with on Linux and it words a null dereference as
  /// `null is not an object`; Chromium says `Cannot read properties of
  /// undefined`. Neither phrase is in the pattern - both are caught by the
  /// `TypeError` prefix alone, which `String(e)` keeps and `e.message` drops.
  /// Nine places in this tree use the `.message` idiom, so this is the assertion
  /// that says which form the callers owe this function.
  it("catches the WebKit and Chromium wordings through the prefix that String(e) keeps", () => {
    const webkit = new TypeError("null is not an object (evaluating 'x.y')");
    const chromium = new TypeError("Cannot read properties of undefined (reading 'invoke')");

    expect(readsAsInternal(String(webkit))).toBe(true);
    expect(readsAsInternal(String(chromium))).toBe(true);

    // ...and the same errors with the prefix stripped are NOT caught, which is
    // exactly why the callers must not strip it.
    expect(readsAsInternal(webkit.message)).toBe(false);
    expect(readsAsInternal(chromium.message)).toBe(false);
  });
});

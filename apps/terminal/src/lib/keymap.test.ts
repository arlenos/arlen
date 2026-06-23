import { describe, it, expect } from "vitest";
import { keyToBytes } from "./keymap";

/// Build a minimal KeyboardEvent-shaped object; keyToBytes only reads these.
function ev(
  key: string,
  mods: { ctrl?: boolean; alt?: boolean; meta?: boolean; shift?: boolean } = {},
): KeyboardEvent {
  return {
    key,
    ctrlKey: !!mods.ctrl,
    altKey: !!mods.alt,
    metaKey: !!mods.meta,
    shiftKey: !!mods.shift,
  } as KeyboardEvent;
}

describe("keyToBytes (raw-PTY input encoding)", () => {
  it("sends a printable character as itself", () => {
    expect(keyToBytes(ev("a"))).toBe("a");
    expect(keyToBytes(ev("Z"))).toBe("Z");
    expect(keyToBytes(ev("1"))).toBe("1");
    expect(keyToBytes(ev(" "))).toBe(" ");
  });

  it("maps Enter to CR and Backspace to DEL", () => {
    expect(keyToBytes(ev("Enter"))).toBe("\r");
    expect(keyToBytes(ev("Backspace"))).toBe("\x7f");
    expect(keyToBytes(ev("Tab"))).toBe("\t");
    expect(keyToBytes(ev("Escape"))).toBe("\x1b");
  });

  it("maps the cursor and navigation keys to their CSI sequences", () => {
    expect(keyToBytes(ev("ArrowUp"))).toBe("\x1b[A");
    expect(keyToBytes(ev("ArrowDown"))).toBe("\x1b[B");
    expect(keyToBytes(ev("ArrowRight"))).toBe("\x1b[C");
    expect(keyToBytes(ev("ArrowLeft"))).toBe("\x1b[D");
    expect(keyToBytes(ev("Home"))).toBe("\x1b[H");
    expect(keyToBytes(ev("End"))).toBe("\x1b[F");
    expect(keyToBytes(ev("Delete"))).toBe("\x1b[3~");
    expect(keyToBytes(ev("PageUp"))).toBe("\x1b[5~");
  });

  it("maps Ctrl+<letter> to its C0 control code", () => {
    expect(keyToBytes(ev("c", { ctrl: true }))).toBe("\x03"); // SIGINT
    expect(keyToBytes(ev("l", { ctrl: true }))).toBe("\x0c"); // clear
    expect(keyToBytes(ev("d", { ctrl: true }))).toBe("\x04"); // EOF
    expect(keyToBytes(ev("a", { ctrl: true }))).toBe("\x01"); // start-of-line
    expect(keyToBytes(ev("C", { ctrl: true }))).toBe("\x03"); // case-insensitive
  });

  it("maps the conventional symbol controls and Ctrl+Space", () => {
    expect(keyToBytes(ev(" ", { ctrl: true }))).toBe("\x00");
    expect(keyToBytes(ev("[", { ctrl: true }))).toBe("\x1b");
    expect(keyToBytes(ev("\\", { ctrl: true }))).toBe("\x1c"); // SIGQUIT
  });

  it("prefixes Alt with ESC (meta)", () => {
    expect(keyToBytes(ev("x", { alt: true }))).toBe("\x1bx");
  });

  it("leaves copy/paste and WM combos to the app (null)", () => {
    expect(keyToBytes(ev("c", { ctrl: true, shift: true }))).toBeNull(); // copy
    expect(keyToBytes(ev("v", { ctrl: true, shift: true }))).toBeNull(); // paste
    expect(keyToBytes(ev("a", { meta: true }))).toBeNull(); // WM/super combo
  });

  it("ignores bare modifier and unknown keys", () => {
    expect(keyToBytes(ev("Shift"))).toBeNull();
    expect(keyToBytes(ev("Control"))).toBeNull();
    expect(keyToBytes(ev("F13"))).toBeNull();
  });
});

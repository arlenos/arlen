import { describe, it, expect } from "vitest";
import { menuNoteFor } from "./menuGuard";

const nothing = { selectedCount: 0, reading: false };
const one = { selectedCount: 1, reading: true };
const several = { selectedCount: 3, reading: false };

describe("menuNoteFor", () => {
  it("asks for a message when nothing is selected", () => {
    for (const a of ["message.reply", "message.forward", "message.archive", "message.delete"]) {
      expect(menuNoteFor(a, nothing)).toBe("ml.menu.needsMessage");
    }
  });

  it("lets every message action through with one open", () => {
    for (const a of ["message.reply", "message.forward", "message.archive", "message.delete"]) {
      expect(menuNoteFor(a, one)).toBeNull();
    }
  });

  it("says reply and forward take one at a time, and lets the bulk moves run", () => {
    expect(menuNoteFor("message.reply", several)).toBe("ml.menu.needsOne");
    expect(menuNoteFor("message.forward", several)).toBe("ml.menu.needsOne");
    expect(menuNoteFor("message.archive", several)).toBeNull();
    expect(menuNoteFor("message.delete", several)).toBeNull();
  });

  it("does not stand in front of a pick that is not about an open message", () => {
    for (const a of ["message.new", "go.inbox", "go.trash", "view.something"]) {
      expect(menuNoteFor(a, nothing)).toBeNull();
    }
  });
});

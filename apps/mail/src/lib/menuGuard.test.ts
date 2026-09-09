import { describe, it, expect } from "vitest";
import { menuNoteFor } from "./menuGuard";

const nothing = { selectedCount: 0, reading: false };
const one = { selectedCount: 1, reading: true };
const several = { selectedCount: 3, reading: false };

describe("menuNoteFor", () => {
  it("asks for a message when nothing is selected, and names the action", () => {
    expect(menuNoteFor("message.reply", nothing)).toEqual({
      key: "ml.menu.needsMessage",
      label: "ml.reply",
    });
    for (const a of ["message.forward", "message.archive", "message.delete"]) {
      expect(menuNoteFor(a, nothing)?.key).toBe("ml.menu.needsMessage");
    }
  });

  /// THE REASON THE ACTION IS IN IT. Without the name the note said "Select a
  /// message first." directly above the reading pane's own "Select a message to
  /// read it." - the same thing twice, eight lines apart, which only a picture
  /// showed. What the pane cannot say is which entry was picked.
  it("names each guarded action by the label its own menu entry uses", () => {
    expect(menuNoteFor("message.forward", nothing)?.label).toBe("ml.forward");
    expect(menuNoteFor("message.archive", nothing)?.label).toBe("ml.archive");
    expect(menuNoteFor("message.delete", nothing)?.label).toBe("ml.delete");
  });

  it("lets every message action through with one open", () => {
    for (const a of ["message.reply", "message.forward", "message.archive", "message.delete"]) {
      expect(menuNoteFor(a, one)).toBeNull();
    }
  });

  it("says reply and forward take one at a time, and lets the bulk moves run", () => {
    expect(menuNoteFor("message.reply", several)?.key).toBe("ml.menu.needsOne");
    expect(menuNoteFor("message.forward", several)?.key).toBe("ml.menu.needsOne");
    expect(menuNoteFor("message.archive", several)).toBeNull();
    expect(menuNoteFor("message.delete", several)).toBeNull();
  });

  it("does not stand in front of a pick that is not about an open message", () => {
    for (const a of ["message.new", "go.inbox", "go.trash", "view.something"]) {
      expect(menuNoteFor(a, nothing)).toBeNull();
    }
  });
});

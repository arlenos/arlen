/// Which menu picks do not apply in the state the window is in, and what to say.
///
/// A menu entry is ALWAYS there. Its toolbar twin only appears once there is
/// something to act on - `showActions` wants a selection and an open message -
/// so the toolbar cannot be pressed in a state it does not apply to and the menu
/// can. Picking Reply with nothing open did nothing and said nothing, which
/// reads exactly like a menu item that was never wired up.
///
/// Pure, so the rule is a test rather than a thing to click through: the window
/// state that decides it (`selected`, `reading`) is component-local, and the
/// only other way to check this is to open the app and pick the item.

/// The state a menu pick is judged against.
export interface MailState {
  /// How many rows are selected. Opening a row selects it, so zero means
  /// nothing at all is in front of the reader.
  selectedCount: number;
  /// Whether one conversation is open in the reading pane. Selecting several
  /// rows clears it, which is what separates the two sentences below.
  reading: boolean;
}

/// What the note says, and the label of the action it is about.
///
/// THE ACTION HAS TO BE IN IT, and the first version of this file left it out.
/// Read in a picture, the note said "Select a message first." directly above the
/// reading pane's own empty state, which already says "Select a message to read
/// it." - two sentences telling somebody the same thing eight lines apart. The
/// fact the pane does NOT carry is which menu entry was just picked and why it
/// did nothing, so that is what the note is for.
export interface MenuNote {
  /// The sentence, as a catalogue key taking an `action` parameter.
  key: string;
  /// The action's own label, as a catalogue key.
  label: string;
}

/// The label each guarded action goes by, in the reader's language. The same
/// keys the menu itself uses, so the note and the menu entry cannot drift apart.
const LABELS: Record<string, string> = {
  "message.reply": "ml.reply",
  "message.forward": "ml.forward",
  "message.archive": "ml.archive",
  "message.delete": "ml.delete",
};

/// The note to show, or null when the pick applies and should run.
export function menuNoteFor(action: string, state: MailState): MenuNote | null {
  const label = LABELS[action];
  if (!label) {
    // Compose, the folder moves, and anything this window does not know are not
    // about a message in front of you. `message.new` carries its own guard (a
    // mailbox with nowhere to send offers no Compose at all).
    return null;
  }
  if (action === "message.reply" || action === "message.forward") {
    if (state.selectedCount === 0) return { key: "ml.menu.needsMessage", label };
    if (!state.reading) return { key: "ml.menu.needsOne", label };
    return null;
  }
  // Archive and delete act on every selected row, so several is fine and none
  // is not.
  return state.selectedCount === 0 ? { key: "ml.menu.needsMessage", label } : null;
}

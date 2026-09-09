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

/// The catalogue key to show, or null when the pick applies and should run.
export function menuNoteFor(action: string, state: MailState): string | null {
  if (action === "message.reply" || action === "message.forward") {
    if (state.selectedCount === 0) return "ml.menu.needsMessage";
    if (!state.reading) return "ml.menu.needsOne";
    return null;
  }
  if (action === "message.archive" || action === "message.delete") {
    // These act on every selected row, so several is fine and none is not.
    return state.selectedCount === 0 ? "ml.menu.needsMessage" : null;
  }
  // Compose, the folder moves, and anything this window does not know are not
  // about a message in front of you. `message.new` carries its own guard (a
  // mailbox with nowhere to send offers no Compose at all).
  return null;
}

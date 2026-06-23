/// Translate a browser KeyboardEvent into the byte string a PTY expects, so the
/// terminal grid can be the interactive surface: every keystroke goes straight
/// to the shell's line editor (zsh `zle`), which is what makes p10k,
/// zsh-syntax-highlighting and autosuggestions render live in the grid (PR-2
/// re-root: raw-PTY input, not a composer textbox that only sends on Enter).
///
/// Returns the bytes to write to the PTY, or `null` when the event is not a
/// terminal keystroke and should be left to the app (copy/paste shortcuts, pure
/// modifier presses, browser/WM combos). The caller sends a non-null result via
/// `terminal_input` and calls `preventDefault()`.
///
/// The encodings follow the de-facto xterm convention (normal cursor mode):
/// printable keys send their character, Enter sends CR (the shell's zle turns it
/// into accept-line), Backspace sends DEL (0x7f), control letters send C0 codes
/// (Ctrl-C = 0x03), Alt sends an ESC prefix (meta), and the navigation keys send
/// their CSI sequences.
export function keyToBytes(event: KeyboardEvent): string | null {
  const { key, ctrlKey, altKey, metaKey, shiftKey } = event;

  // Copy/paste and other Ctrl+Shift / Meta combos are app/WM shortcuts, never
  // terminal input - a terminal's Ctrl+Shift+C is copy, not SIGINT.
  if (metaKey) return null;
  if (ctrlKey && shiftKey) return null;

  // Named (non-printable) keys first, so e.g. "Enter" is not treated as text.
  const named = namedKey(key);
  if (named !== undefined) return named;

  // A single printable character. Ctrl+<letter> is a C0 control code; Alt is a
  // meta (ESC) prefix; a bare character is itself.
  if (key.length === 1) {
    if (ctrlKey) {
      const code = controlCode(key);
      return code === null ? null : code;
    }
    return altKey ? "\x1b" + key : key;
  }

  // Unknown key (a bare modifier, a media key, F13+, ...): not terminal input.
  return null;
}

/// The byte sequence for a named key, or `undefined` if `key` is not one (i.e.
/// it is a printable character handled by the caller). `\r` for Enter is the
/// carriage return zle expects; `\x7f` (DEL) is the conventional Backspace.
function namedKey(key: string): string | undefined {
  switch (key) {
    case "Enter":
      return "\r";
    case "Backspace":
      return "\x7f";
    case "Tab":
      return "\t";
    case "Escape":
      return "\x1b";
    case "ArrowUp":
      return "\x1b[A";
    case "ArrowDown":
      return "\x1b[B";
    case "ArrowRight":
      return "\x1b[C";
    case "ArrowLeft":
      return "\x1b[D";
    case "Home":
      return "\x1b[H";
    case "End":
      return "\x1b[F";
    case "PageUp":
      return "\x1b[5~";
    case "PageDown":
      return "\x1b[6~";
    case "Insert":
      return "\x1b[2~";
    case "Delete":
      return "\x1b[3~";
    default:
      return undefined;
  }
}

/// The C0 control code for Ctrl+<char>, or `null` if the combination has none.
/// Letters map to 0x01-0x1a (Ctrl-A = 0x01); the conventional symbol controls
/// cover Ctrl-Space/@ (NUL), and the [ \ ] ^ _ block (0x1b-0x1f).
function controlCode(char: string): string | null {
  const lower = char.toLowerCase();
  if (lower >= "a" && lower <= "z") {
    return String.fromCharCode(lower.charCodeAt(0) - 96); // 'a'(97) -> 1
  }
  switch (char) {
    case " ":
    case "@":
      return "\x00";
    case "[":
      return "\x1b";
    case "\\":
      return "\x1c";
    case "]":
      return "\x1d";
    case "^":
      return "\x1e";
    case "_":
      return "\x1f";
    default:
      return null;
  }
}

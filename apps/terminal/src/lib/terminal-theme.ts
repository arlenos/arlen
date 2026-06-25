/// The Arlen terminal palette as an xterm.js `ITheme` (themed via the theme
/// object, never CSS - xterm paints the grid on a canvas, so colours must reach
/// it as the options object the engine hands to `new Terminal(...)`).
///
/// The 16 ANSI slots are the Arlen muted set: desaturated, soft, kitty-modern,
/// good contrast without xterm's hard pure colours. The surface and text come
/// from the Arlen app theme (`sdk/theme` dark: bg.app, a soft foreground), not
/// raw black. These are the shipped defaults; once the theme system projects
/// `terminal.ansi` to the frontend, build the ITheme from the live theme so a
/// re-skin follows (the same 16 slots also feed the GTK/Qt/kitty generators -
/// author them into `[terminal.ansi]` so every surface agrees).
export interface XtermTheme {
  background: string;
  foreground: string;
  cursor: string;
  cursorAccent: string;
  selectionBackground: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
}

/// The Arlen terminal theme (xterm.js ITheme-shaped).
export const arlenTerminalTheme: XtermTheme = {
  // The app surface, not raw black; a soft light text, not harsh white.
  background: "#0f0f0f",
  foreground: "#e4e5ea",
  // A soft, visible block cursor; the glyph under it takes the surface colour.
  cursor: "#d4d4d8",
  cursorAccent: "#0f0f0f",
  // A quiet selection wash in the foreground register.
  selectionBackground: "#33353d",
  // ANSI 0-7: the muted Arlen set.
  black: "#15161b",
  red: "#c96a6a",
  green: "#8fae74",
  yellow: "#d4b483",
  blue: "#7d9cc4",
  magenta: "#b08bc4",
  cyan: "#83b3b1",
  white: "#c8c9cf",
  // ANSI 8-15: the brights, a touch lighter, still soft.
  brightBlack: "#54565e",
  brightRed: "#d98585",
  brightGreen: "#a6c98a",
  brightYellow: "#e3c99a",
  brightBlue: "#97b5da",
  brightMagenta: "#c4a0d6",
  brightCyan: "#9bcac8",
  brightWhite: "#f2f3f7",
};

/// The terminal mono: the bundled soft face first, system fallbacks after. xterm
/// measures the font, so it must be loaded (the `@fontsource/cascadia-code`
/// import in `app.css`) before the grid sizes - await `document.fonts.ready`.
export const TERMINAL_FONT_FAMILY =
  "'Cascadia Code', 'JetBrainsMono Nerd Font Mono', 'JetBrains Mono', ui-monospace, monospace";

/// The cell font size, derived from the base size token (PR-3, 14px) - a
/// comfortable default, never a tiny hardcode. A px number, as xterm wants.
export const TERMINAL_FONT_SIZE = 14;

/// Line height as a multiple - a touch of air over the tight 1.0 default for a
/// softer, more readable grid, without the airy 1.5 of the chrome labels.
export const TERMINAL_LINE_HEIGHT = 1.2;

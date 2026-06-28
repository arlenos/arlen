/// Syntax highlighting for code artifacts, via Shiki on a restrained Arlen
/// theme. The palette mirrors the terminal's muted ANSI set
/// (`apps/terminal/src/lib/terminal-theme.ts` `arlenTerminalTheme`), so code
/// artifacts and terminal output share one soft, flat syntax-colour family
/// instead of a vibrant web theme. (The eventual single source is the Arlen
/// theme system; these literals mirror it until that lands.) Highlighting is
/// async + best-effort: the caller renders plain monospace first and upgrades
/// when this resolves; an unknown language or any failure stays plain.
import { createHighlighter, type Highlighter } from "shiki";

/// The muted Arlen palette (mirrors `arlenTerminalTheme`).
const C = {
  fg: "#e4e5ea",
  muted: "#54565e",
  red: "#c96a6a",
  green: "#8fae74",
  yellow: "#d4b483",
  blue: "#7d9cc4",
  cyan: "#83b3b1",
  white: "#c8c9cf",
};

/// A Shiki theme over the muted palette. Background is left to the frame (the
/// `.shiki` element's background is overridden to transparent in CSS), so the
/// code block sits on the artifact frame, not a Shiki-coloured box.
const ARLEN_THEME = {
  name: "arlen",
  type: "dark" as const,
  colors: { "editor.background": "#0f0f0f", "editor.foreground": C.fg },
  settings: [
    { scope: ["comment", "punctuation.definition.comment"], settings: { foreground: C.muted, fontStyle: "italic" } },
    { scope: ["string", "constant.other.symbol", "string.regexp"], settings: { foreground: C.green } },
    { scope: ["constant.numeric", "constant.language", "constant.character"], settings: { foreground: C.yellow } },
    { scope: ["keyword", "storage", "storage.type", "keyword.control", "modifier"], settings: { foreground: C.red } },
    { scope: ["keyword.operator"], settings: { foreground: C.white } },
    { scope: ["entity.name.function", "support.function", "meta.function-call"], settings: { foreground: C.blue } },
    { scope: ["entity.name.type", "support.type", "support.class", "entity.name.class"], settings: { foreground: C.cyan } },
    { scope: ["variable", "variable.parameter", "meta.definition.variable"], settings: { foreground: C.fg } },
    { scope: ["entity.name.tag", "punctuation.definition.tag"], settings: { foreground: C.red } },
    { scope: ["entity.other.attribute-name"], settings: { foreground: C.yellow } },
    { scope: ["punctuation", "meta.brace"], settings: { foreground: C.white } },
  ],
};

let hlPromise: Promise<Highlighter> | null = null;
const loaded = new Set<string>();

async function getHighlighter(): Promise<Highlighter> {
  if (!hlPromise) {
    hlPromise = createHighlighter({ themes: [ARLEN_THEME], langs: [] });
  }
  return hlPromise;
}

/// Load a language grammar on demand. Returns whether it is available; an
/// unknown id resolves to false (the caller then renders plaintext).
async function ensureLanguage(hl: Highlighter, lang: string): Promise<boolean> {
  if (loaded.has(lang)) return true;
  try {
    await hl.loadLanguage(lang as Parameters<Highlighter["loadLanguage"]>[0]);
    loaded.add(lang);
    return true;
  } catch {
    return false;
  }
}

/// Highlight `code` to Shiki HTML on the Arlen theme, or return null on failure
/// (the caller keeps its plain-monospace fallback). `lang` is the payload hint;
/// an unknown one falls back to plaintext rather than failing.
export async function highlightCode(code: string, lang?: string): Promise<string | null> {
  try {
    const hl = await getHighlighter();
    const language = lang && (await ensureLanguage(hl, lang)) ? lang : "text";
    return hl.codeToHtml(code, { lang: language, theme: "arlen" });
  } catch {
    return null;
  }
}

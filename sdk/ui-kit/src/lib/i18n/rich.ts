// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// Sentences that carry inline markup, without cutting them into fragments.
///
/// The runtime beside this file states the rule: messages are grammatically whole,
/// never concatenated fragments. Three sites break it today, all the same way - a
/// sentence with a `<code>` or `<strong>` in the middle gets split at the markup:
///
///     {$t("s.monitor.noDisplays.pre")} <code>wlr-output-management</code> {$t("s.monitor.noDisplays.post")}
///
/// A translator receives `.pre` and `.post` as two entries and never sees the
/// sentence. They cannot move the term - German puts the verb at the end, Japanese
/// puts the object first - so the only translation that fits is one with English word
/// order. The fragments are also untranslatable on their own: `.post` is often a bare
/// clause ending with a full stop, which several languages inflect differently
/// depending on what preceded it.
///
/// So the message stays whole and the markup becomes a placeholder in it:
///
///     "s.monitor.noDisplays": "No displays reported. The compositor may not expose {$tool}."
///
/// The caller formats it with `mark("tool")` in place of the value, and this splits
/// the formatted string back into parts so the render site can wrap that one part in
/// whatever element it wants. The translator moves `{$tool}` wherever their grammar
/// needs it and the styling follows it, because the split reads the formatted output
/// rather than assuming a position.
///
/// Marks are named rather than positional for that same reason: a translator who
/// swaps two placeholders would otherwise get the two elements swapped with them.

/// Private-use codepoints, so a mark cannot collide with anything a catalog or a
/// user-supplied value legitimately contains. U+E000..U+F8FF is the Basic
/// Multilingual Plane's private use area: unassigned by Unicode forever, and not
/// typeable, so no real message text produces this by accident.
const OPEN = "\uE000";
const CLOSE = "\uE001";

/// The value to format a message with in place of a marked-up term. The `name` is
/// how the render site names the snippet that styles it.
export function mark(name: string): string {
  return OPEN + name + CLOSE;
}

/// One piece of a formatted message: literal text, or a named hole to render an
/// element into.
export type RichPart =
  | { kind: "text"; text: string }
  | { kind: "mark"; name: string };

/// Split a formatted message into its literal and marked parts, in the order they
/// appear in the output - which is the order the translation put them in, not the
/// order the arguments were passed.
///
/// Never throws and never drops text. A stray `OPEN` with no `CLOSE` (a corrupt
/// catalog, or a value that somehow carried one) is emitted as literal text, so the
/// worst case is one visible odd character rather than a sentence that silently
/// loses its tail.
export function richParts(text: string): RichPart[] {
  const parts: RichPart[] = [];
  let at = 0;

  while (at < text.length) {
    const open = text.indexOf(OPEN, at);
    if (open === -1) break;
    const close = text.indexOf(CLOSE, open + 1);
    if (close === -1) break; // unterminated: the rest is literal

    if (open > at) parts.push({ kind: "text", text: text.slice(at, open) });
    parts.push({ kind: "mark", name: text.slice(open + 1, close) });
    at = close + 1;
  }

  if (at < text.length) parts.push({ kind: "text", text: text.slice(at) });
  return parts;
}

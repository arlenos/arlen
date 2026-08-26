/// The two places this window turns data into a sentence, out where they can be
/// tested and where a redesign can pick them up.
///
/// Both were private functions inside `+page.svelte`. That is a fine place for
/// markup glue and a poor one for rules - "which sentence does a calendar part
/// get" and "what does a Date header look like to a reader" are decisions with
/// right and wrong answers, and neither could be exercised without rendering the
/// component. The window's LOOK is being redone by another lane; these rules are
/// not, so they should not be sitting in the file that is going away.

/// A translator: the message id and its values in, the sentence out.
export type Translate = (key: string, values?: Record<string, unknown>) => string;

/// The sentence for a calendar part, chosen by what the message claims it is.
///
/// The method is a protocol token (`REQUEST`, `CANCEL`, ...) and the reader is
/// not the protocol, so each known one has its own sentence in the catalogue. An
/// unknown method is shown AS WRITTEN rather than dropped: a part marked
/// something this app has never heard of is exactly when a person wants to see
/// the word and decide for themselves.
///
/// The ids are spelled out rather than built with `ml.invitation.${method}`,
/// because the key gate reads LITERAL keys and a composed one is invisible to
/// it - a rename would take the sentence away with nothing failing.
export function invitationWords(method: string | null, t: Translate): string {
  if (method === null) return t("ml.invitation.unmarked");
  if (method === "request") return t("ml.invitation.request");
  if (method === "cancel") return t("ml.invitation.cancel");
  if (method === "reply") return t("ml.invitation.reply");
  if (method === "publish") return t("ml.invitation.publish");
  return t("ml.invitation.other", { method });
}

/// A size written the way the reader writes one: `84.2 KB` in English,
/// `84,2 KB` in German. Moved here from the page when the reading surface grew
/// components - a rule, not markup glue.
///
/// THE UNIT IS OURS, THE NUMBER IS THE LOCALE'S, and it took a German render to
/// see why. This asked `Intl` for `style: "unit", unit: "byte"` with
/// `notation: "compact"`, which does not mean the same thing in every language:
/// English compacts bytes to `84.2KB`, German does not compact them at all
/// below a million and then says `5,2 Mio. B` - "5.2 million bytes", which no
/// file manager in either language writes. Worse at the size in the frame:
/// 84213 came out `84.213 B`, where the period is a German THOUSANDS separator
/// and reads to anyone else as eighty-four point two bytes.
///
/// So the scale step is ours - the same 1000-based ladder the English output
/// already used, so nothing changes for that reader but the space before the
/// unit the doc always claimed - and only the number goes through `Intl`, which
/// is the part that genuinely differs by language (`84.2` against `84,2`). The
/// unit names are written the same in both.
///
/// `KB`, NOT the SI-correct `kB`, because the rest of this system says `KB`:
/// `@arlen/ui-kit`'s `formatSize` (the files browser, the duplicate finder, the
/// info panel) and the shell's process list both do. A capital K is kelvin and a
/// pedant is right about that, but a person who sees `84,2 kB` in a message and
/// `84,2 KB` for the same file in the browser learns nothing except that the two
/// were written by different hands. The kit is the house standard; this follows
/// it. The 1000-vs-1024 split is left alone deliberately: the kit and this are
/// 1000-based for FILE sizes, the shell is 1024-based for process MEMORY, which
/// is what every task manager does.
const SIZE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

export function formatBytes(n: number, loc: string): string {
  let value = Math.max(0, n);
  let step = 0;
  while (value >= 1000 && step < SIZE_UNITS.length - 1) {
    value /= 1000;
    step += 1;
  }
  // Bytes are whole things; a scaled size wants one place. `1000 B` never
  // appears because the loop takes it up a step first.
  const digits = step === 0 ? 0 : 1;
  const number = new Intl.NumberFormat(loc, { maximumFractionDigits: digits }).format(value);
  return `${number} ${SIZE_UNITS[step]}`;
}

/// The display half of a mailbox line: `Mara Winter <mara@example.org>` reads
/// as the name, a bare address as itself. The full line stays wherever trust
/// matters; this is for the avatar and the list row.
export function displayName(from: string): string {
  const at = from.indexOf("<");
  const name = (at > 0 ? from.slice(0, at) : from).trim();
  return name || from;
}

/// The address half of a mailbox line: `Mara <mara@example.org>` yields the
/// address, a bare address yields itself, anything else nothing.
export function addressOf(from: string): string | null {
  const angled = from.match(/<([^<>\s]+@[^<>\s]+)>/);
  if (angled) return angled[1].toLowerCase();
  const bare = from.trim();
  return /^[^\s]+@[^\s]+$/.test(bare) ? bare.toLowerCase() : null;
}

/// The conversation key for a subject: the reply/forward prefixes stripped,
/// case folded. "Re: Re: AW: Plans" and "plans" are one conversation. This is
/// the pragmatic subject-threading a client can do without the References
/// chain; real RFC threading is the backend's to add (mail-app.md §7 lists
/// threading algorithms as unresearched).
export function threadKey(subject: string | null): string {
  let s = (subject ?? "").trim();
  const prefix = /^(re|fwd|fw|aw|wg)\s*(\[\d+\])?\s*:\s*/i;
  while (prefix.test(s)) s = s.replace(prefix, "");
  // A GROUPING KEY, never shown. Lowercased like every other key this returns,
  // and the list writes its own "(kein Betreff)" from the catalog when a row has
  // no subject to show. Translating this would split one thread into one per
  // language the reader has ever used.
  return s.toLowerCase() || "(no subject)";
}

/// The Date header, written the way the reader's language writes one.
///
/// A header that will not parse is returned VERBATIM. The raw line is what the
/// sender wrote and it is the only thing anybody has for a malformed date;
/// showing an empty field, or today's date, would both be inventions.
export function formatSent(raw: string, loc: string): string {
  const at = new Date(raw);
  if (Number.isNaN(at.getTime())) return raw;
  return new Intl.DateTimeFormat(loc, { dateStyle: "long", timeStyle: "short" }).format(at);
}

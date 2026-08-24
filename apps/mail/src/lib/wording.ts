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

/// A size written the way the reader writes one: `16 kB` in English, a comma
/// decimal in German, the unit from the reader's locale. Moved here from the
/// page when the reading surface grew components - a rule, not markup glue.
export function formatBytes(n: number, loc: string): string {
  return new Intl.NumberFormat(loc, {
    style: "unit",
    unit: "byte",
    unitDisplay: "narrow",
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(n);
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

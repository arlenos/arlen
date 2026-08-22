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

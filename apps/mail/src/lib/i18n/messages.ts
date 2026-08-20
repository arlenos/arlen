/// The mail app's message catalog (MF2, en + de). Keys are prefixed `ml.` per
/// the per-app convention.
///
/// Born translatable: every sentence a person reads is a key here, in both
/// languages, from the first commit.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: {
    "ml.app.title": "Mail",
    "ml.nothingOpen": "No message is open. Open a message from the file manager, or pass one on the command line.",
    "ml.failed": "Could not read that message: {$reason}",
    "ml.from": "From",
    "ml.subject": "Subject",
    "ml.date": "Date",
    "ml.unsigned": "as written by the sender, not verified",
    "ml.noText": "This message has no plain-text part.",
    // Not a missing feature, and the sentence says which. See the module note in
    // the app's lib.rs: containing the renderer does not stop the message
    // calling home, so nothing renders it yet.
    "ml.htmlNotShown": "This message also has an HTML part. It is not shown: displaying it safely is not built yet, and showing it unsafely would let the message report back that you read it.",
    "ml.divergence": "The plain-text and HTML parts do not say the same thing. {$detail}",
    "ml.refused": "This message contradicts itself about its own format, so it is not being interpreted: {$reason}",
    "ml.channels": "Headers in this message ask to report back: {$list}",
  },
  de: {
    "ml.app.title": "E-Mail",
    "ml.nothingOpen": "Keine Nachricht geöffnet. Öffne eine aus der Dateiverwaltung, oder übergib eine auf der Kommandozeile.",
    "ml.failed": "Diese Nachricht war nicht lesbar: {$reason}",
    "ml.from": "Von",
    "ml.subject": "Betreff",
    "ml.date": "Datum",
    "ml.unsigned": "so geschrieben von der absendenden Seite, nicht geprüft",
    "ml.noText": "Diese Nachricht hat keinen reinen Textteil.",
    "ml.htmlNotShown": "Diese Nachricht hat auch einen HTML-Teil. Er wird nicht gezeigt: ihn sicher darzustellen ist noch nicht gebaut, und ihn unsicher zu zeigen würde der Nachricht erlauben zu melden, dass du sie gelesen hast.",
    "ml.divergence": "Der Textteil und der HTML-Teil sagen nicht dasselbe. {$detail}",
    "ml.refused": "Diese Nachricht widerspricht sich über ihr eigenes Format, deshalb wird sie nicht interpretiert: {$reason}",
    "ml.channels": "Header in dieser Nachricht wollen zurückmelden: {$list}",
  },
};

/// Translate by key. Every sentence in this app goes through here.
export const t = createTranslator(messages);

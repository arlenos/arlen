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
    "ml.to": "To",
    "ml.cc": "Copied",
    "ml.unsigned": "as written by the sender, not verified",
    "ml.noText": "This message has no plain-text part.",
    // Not a missing feature, and the sentence says which. See the module note in
    // the app's lib.rs: containing the renderer does not stop the message
    // calling home, so nothing renders it yet.
    "ml.htmlNotShown": "This message also has an HTML part. It is not shown: displaying it safely is not built yet, and showing it unsafely would let the message report back that you read it.",
    "ml.divergenceBoth": "The plain-text and formatted versions of this message differ. Only in the text: {$text}. Only in the formatting: {$html}.",
    "ml.divergenceText": "The plain-text version says things the formatted one does not: {$text}.",
    "ml.divergenceHtml": "The formatted version says things the plain-text one does not: {$html}.",
    "ml.refused": "This message contradicts itself about its own format, so it is not being interpreted: {$reason}",
    "ml.channels": "Headers in this message ask to report back: {$list}",
    // Named and measured, never opened. Same principle as the HTML notice: say
    // what the message carries without acting on it.
    // MF2 (`.input`/`.match`), not ICU MessageFormat 1. The first version of this
    // used `{$count, plural, one {...} other {...}}` and the window printed that
    // string verbatim: the formatter under this catalog is MessageFormat 2, where
    // a selector is a declaration and not an inline argument. Rendered before it
    // was believed, which is how it was caught.
    "ml.carries":
      ".input {$count :number}\n.match $count\none {{This message carries one file, not opened:}}\n* {{This message carries {$count} files, not opened:}}",
    "ml.attachment": "{$name}, {$type}, {$size}",
    "ml.unnamedAttachment": "a file the sender did not name",
  },
  de: {
    "ml.app.title": "E-Mail",
    "ml.nothingOpen": "Keine Nachricht geöffnet. Öffne eine aus der Dateiverwaltung, oder übergib eine auf der Kommandozeile.",
    "ml.failed": "Diese Nachricht war nicht lesbar: {$reason}",
    "ml.from": "Von",
    "ml.subject": "Betreff",
    "ml.date": "Datum",
    "ml.to": "An",
    "ml.cc": "Kopie",
    "ml.unsigned": "so geschrieben von der absendenden Seite, nicht geprüft",
    "ml.noText": "Diese Nachricht hat keinen reinen Textteil.",
    "ml.htmlNotShown": "Diese Nachricht hat auch einen HTML-Teil. Er wird nicht gezeigt: ihn sicher darzustellen ist noch nicht gebaut, und ihn unsicher zu zeigen würde der Nachricht erlauben zu melden, dass du sie gelesen hast.",
    "ml.divergenceBoth": "Der Textteil und der formatierte Teil dieser Nachricht unterscheiden sich. Nur im Text: {$text}. Nur in der Formatierung: {$html}.",
    "ml.divergenceText": "Der Textteil sagt Dinge, die der formatierte Teil nicht sagt: {$text}.",
    "ml.divergenceHtml": "Der formatierte Teil sagt Dinge, die der Textteil nicht sagt: {$html}.",
    "ml.refused": "Diese Nachricht widerspricht sich über ihr eigenes Format, deshalb wird sie nicht interpretiert: {$reason}",
    "ml.channels": "Header in dieser Nachricht wollen zurückmelden: {$list}",
    "ml.carries":
      ".input {$count :number}\n.match $count\none {{Diese Nachricht trägt eine Datei bei sich, nicht geöffnet:}}\n* {{Diese Nachricht trägt {$count} Dateien bei sich, nicht geöffnet:}}",
    "ml.attachment": "{$name}, {$type}, {$size}",
    "ml.unnamedAttachment": "eine Datei, die die absendende Seite nicht benannt hat",
  },
};

/// Translate by key. Every sentence in this app goes through here.
export const t = createTranslator(messages);

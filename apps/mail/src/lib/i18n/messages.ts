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
    "ml.nothingOpen": "No message is open. Open a message from Files, or pass one on the command line.",
    // Two named causes rather than whatever the layer below said. `other` stays
    // for a failure the command cannot name, which today means the host itself
    // is absent - a state a person never meets.
    "ml.failed.unreadable": "That file could not be read, so the message is not shown: {$why}",
    "ml.failed.notAMessage": "That file is not a message, so there is nothing to show. A message file usually ends in .eml.",
    "ml.failed.other": "Could not read that message: {$reason}",
    "ml.from": "From",
    "ml.subject": "Subject",
    "ml.date": "Date",
    "ml.to": "To",
    "ml.cc": "Copied",
    "ml.unsigned": "as written by the sender, not verified",
    "ml.noText": "This message has no plain-text part.",
    "ml.sealed.pgp": "This message is encrypted with PGP. Nothing here can open it, so there is nothing to show.",
    "ml.sealed.smime": "This message is sealed with S/MIME. Nothing here can open it, so there is nothing to show.",
    "ml.sealed.unknown": "This message says it is encrypted, in a way this app does not recognise, so there is nothing to show.",
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
    // A `text/calendar` part, NAMED and not read. The core does not parse the
    // payload - who processes iTIP is an open call between this app and the
    // calendar daemon - so these say what the message CLAIMS the part is for and
    // then say plainly that nothing here opened it. The methods are spelled out
    // in words rather than printed raw: "REQUEST" is a protocol token and the
    // person reading their mail is not the protocol.
    "ml.invitation.request": "This message carries an invitation. Nothing here has read it.",
    "ml.invitation.cancel": "This message carries a cancellation for an event. Nothing here has read it.",
    "ml.invitation.reply": "This message carries somebody's reply to an invitation. Nothing here has read it.",
    "ml.invitation.publish": "This message carries a published event. Nothing here has read it.",
    "ml.invitation.other": "This message carries a calendar part marked {$method}. Nothing here has read it.",
    "ml.invitation.unmarked": "This message carries a calendar part. It does not say what for, and nothing here has read it.",
    "ml.folder.inbox": "Inbox",
    "ml.folder.sent": "Sent",
    "ml.folder.drafts": "Drafts",
    "ml.folder.archive": "Archive",
    "ml.folder.trash": "Trash",
    "ml.compose": "New message",
    "ml.sample": "Example mailbox - no account is connected yet.",
    "ml.unconnected": "No account is connected yet. A message opened from Files still shows here.",
    "ml.search": "Search messages",
    "ml.filter.all": "All",
    "ml.filter.unread": "Unread",
    "ml.noUnread": "Nothing unread here.",
    "ml.emptyFolder": "Nothing in this folder.",
    "ml.noMatch": "Nothing matches your search.",
    "ml.noneSelected": "Select a message to read it.",
    "ml.selectedCount": ".input {$n :number}\n.match $n\none {{One message selected.}}\n* {{{$n} messages selected.}}",
    "ml.threadCount": "{$n} messages in this conversation",
    "ml.openedFile": "Opened file",
    "ml.reply": "Reply",
    "ml.forward": "Forward",
    "ml.archive": "Archive",
    "ml.delete": "Delete",
    "ml.send": "Send",
    "ml.compose.title": "New message",
    "ml.compose.to": "To",
    "ml.compose.subject": "Subject",
    "ml.compose.body": "Write your message",
    "ml.compose.cantSend": "Sending is not connected to an account yet, so this stays in Drafts.",
    "ml.compose.saved": "Saved to Drafts.",
    "ml.compose.discard": "Discard",
  },
  de: {
    "ml.app.title": "E-Mail",
    "ml.nothingOpen": "Keine Nachricht geöffnet. Öffne eine aus Dateien, oder übergib eine auf der Kommandozeile.",
    "ml.failed.unreadable": "Diese Datei konnte nicht gelesen werden, deshalb wird die Nachricht nicht gezeigt: {$why}",
    "ml.failed.notAMessage": "Diese Datei ist keine Nachricht, es gibt also nichts zu zeigen. Eine Nachrichtendatei endet meist auf .eml.",
    "ml.failed.other": "Diese Nachricht war nicht lesbar: {$reason}",
    "ml.from": "Von",
    "ml.subject": "Betreff",
    "ml.date": "Datum",
    "ml.to": "An",
    "ml.cc": "Kopie",
    "ml.unsigned": "so geschrieben von der absendenden Seite, nicht geprüft",
    "ml.noText": "Diese Nachricht hat keinen reinen Textteil.",
    "ml.sealed.pgp": "Diese Nachricht ist mit PGP verschlüsselt. Hier kann sie nichts öffnen, es gibt also nichts zu zeigen.",
    "ml.sealed.smime": "Diese Nachricht ist mit S/MIME versiegelt. Hier kann sie nichts öffnen, es gibt also nichts zu zeigen.",
    "ml.sealed.unknown": "Diese Nachricht sagt, sie sei verschlüsselt, auf eine Art, die diese App nicht kennt. Es gibt also nichts zu zeigen.",
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
    "ml.invitation.request": "Diese Nachricht enthält eine Einladung. Diese App hat sie nicht gelesen.",
    "ml.invitation.cancel": "Diese Nachricht enthält eine Absage zu einem Termin. Diese App hat sie nicht gelesen.",
    "ml.invitation.reply": "Diese Nachricht enthält jemandes Antwort auf eine Einladung. Diese App hat sie nicht gelesen.",
    "ml.invitation.publish": "Diese Nachricht enthält einen veröffentlichten Termin. Diese App hat ihn nicht gelesen.",
    "ml.invitation.other": "Diese Nachricht enthält einen Kalenderteil, gekennzeichnet als {$method}. Diese App hat ihn nicht gelesen.",
    "ml.invitation.unmarked": "Diese Nachricht enthält einen Kalenderteil. Wofür, sagt sie nicht, und diese App hat ihn nicht gelesen.",
    "ml.folder.inbox": "Posteingang",
    "ml.folder.sent": "Gesendet",
    "ml.folder.drafts": "Entwürfe",
    "ml.folder.archive": "Archiv",
    "ml.folder.trash": "Papierkorb",
    "ml.compose": "Neue Nachricht",
    "ml.sample": "Beispiel-Postfach, noch kein Konto verbunden.",
    "ml.unconnected": "Noch kein Konto verbunden. Eine aus Dateien geöffnete Nachricht erscheint trotzdem hier.",
    "ml.search": "Nachrichten durchsuchen",
    "ml.filter.all": "Alle",
    "ml.filter.unread": "Ungelesen",
    "ml.noUnread": "Hier ist nichts ungelesen.",
    "ml.emptyFolder": "Nichts in diesem Ordner.",
    "ml.noMatch": "Nichts passt zu deiner Suche.",
    "ml.noneSelected": "Wähl eine Nachricht zum Lesen.",
    "ml.selectedCount": ".input {$n :number}\n.match $n\none {{Eine Nachricht ausgewählt.}}\n* {{{$n} Nachrichten ausgewählt.}}",
    "ml.threadCount": "{$n} Nachrichten in dieser Unterhaltung",
    "ml.openedFile": "Geöffnete Datei",
    "ml.reply": "Antworten",
    "ml.forward": "Weiterleiten",
    "ml.archive": "Archivieren",
    "ml.delete": "Löschen",
    "ml.send": "Senden",
    "ml.compose.title": "Neue Nachricht",
    "ml.compose.to": "An",
    "ml.compose.subject": "Betreff",
    "ml.compose.body": "Schreib deine Nachricht",
    "ml.compose.cantSend": "Senden ist noch mit keinem Konto verbunden, deshalb bleibt das in den Entwürfen.",
    "ml.compose.saved": "In die Entwürfe gelegt.",
    "ml.compose.discard": "Verwerfen",
  },
};

/// Translate by key. Every sentence in this app goes through here.
export const t = createTranslator(messages);

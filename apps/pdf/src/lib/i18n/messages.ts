/// The document reader's message catalog (MF2, en + de). Keys are prefixed
/// `pdf.` per the per-app convention.
///
/// Born translatable: every sentence a person reads is a key here, in both
/// languages, from the first commit. Retrofitting this is the expensive way, and
/// rendering the German build is what catches the sentences that only sound
/// right in English.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: {
    "pdf.app.title": "Document",
    "pdf.contents": "Contents",
    "pdf.noContents": "This document carries no table of contents.",
    "pdf.pages": "{$count} pages",
    "pdf.onePage": "1 page",
    "pdf.page": "Page {$number}",
    "pdf.pageOf": "Page {$number} of {$total}",
    "pdf.search.label": "Search in document",
    "pdf.search.none": "No page contains that.",
    "pdf.search.unsearchable":
      "{$count} pages could not be read, so anything on them is missing from these results.",
    "pdf.nothingOpen": "No document is open. Open a PDF from Files, or pass one on the command line.",
    "pdf.failed": "Could not open this document: {$reason}",
    "pdf.launchUnknown":
      "The reader could not find out which document it was asked to open, so it opened none: {$reason}",
    "pdf.locked": "This document is locked with a password. This reader cannot ask for one yet, so it cannot be opened here.",
    "pdf.menu.view": "View",
    "pdf.menu.go": "Go",
    "pdf.minimize": "Minimize",
    "pdf.close": "Close",
    "pdf.sample": "Example document - nothing on this machine has this name.",
    "pdf.readingView": "Reading view",
    "pdf.documentOnly": "Document only",
    "pdf.zoomIn": "Zoom in",
    "pdf.zoomOut": "Zoom out",
    "pdf.actualSize": "Actual size",
    "pdf.fitWidth": "Fit width",
    "pdf.fitPage": "Fit page",
    "pdf.goToPage": "Go to page",
    "pdf.prevPage": "Previous page",
    "pdf.nextPage": "Next page",
    "pdf.firstPage": "First page",
    "pdf.lastPage": "Last page",
    "pdf.showContents": "Show contents",
    "pdf.hostAbsent": "The reader reads documents through its own host, which is not running here.",
    "pdf.pageFailed": "This page could not be drawn: {$reason}",
    "pdf.textInstead": "The text of this page, without its layout:",
  },
  de: {
    "pdf.app.title": "Dokument",
    "pdf.contents": "Inhalt",
    "pdf.noContents": "Dieses Dokument hat kein Inhaltsverzeichnis.",
    "pdf.pages": "{$count} Seiten",
    "pdf.onePage": "1 Seite",
    "pdf.page": "Seite {$number}",
    "pdf.pageOf": "Seite {$number} von {$total}",
    "pdf.search.label": "Im Dokument suchen",
    "pdf.search.none": "Keine Seite enthält das.",
    "pdf.search.unsearchable":
      "{$count} Seiten konnten nicht gelesen werden, deshalb fehlt alles darauf in diesen Ergebnissen.",
    "pdf.nothingOpen":
      "Es ist kein Dokument geöffnet. Öffne ein PDF aus Dateien oder übergib eins auf der Kommandozeile.",
    "pdf.failed": "Dieses Dokument konnte nicht geöffnet werden: {$reason}",
    "pdf.launchUnknown":
      "Der Reader konnte nicht herausfinden, welches Dokument er öffnen sollte, deshalb hat er keines geöffnet: {$reason}",
    "pdf.locked": "Dieses Dokument ist mit einem Passwort gesperrt. Dieser Leser kann noch keins abfragen, also lässt es sich hier nicht öffnen.",
    "pdf.menu.view": "Ansicht",
    "pdf.menu.go": "Gehe zu",
    "pdf.minimize": "Minimieren",
    "pdf.close": "Schließen",
    "pdf.sample": "Beispieldokument, nichts auf diesem Rechner trägt diesen Namen.",
    "pdf.readingView": "Leseansicht",
    "pdf.documentOnly": "Nur das Dokument",
    "pdf.zoomIn": "Vergrößern",
    "pdf.zoomOut": "Verkleinern",
    "pdf.actualSize": "Originalgröße",
    "pdf.fitWidth": "An Breite anpassen",
    "pdf.fitPage": "Ganze Seite",
    "pdf.goToPage": "Zu Seite springen",
    "pdf.prevPage": "Vorherige Seite",
    "pdf.nextPage": "Nächste Seite",
    "pdf.firstPage": "Erste Seite",
    "pdf.lastPage": "Letzte Seite",
    "pdf.showContents": "Inhalt anzeigen",
    "pdf.hostAbsent": "Der Reader liest Dokumente über seinen eigenen Host, der hier nicht läuft.",
    "pdf.pageFailed": "Diese Seite konnte nicht gezeichnet werden: {$reason}",
    "pdf.textInstead": "Der Text dieser Seite, ohne ihr Layout:",
  },
};

export const t = createTranslator(messages);

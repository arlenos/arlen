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
    "pdf.pageFlow": "The document's pages",
    "pdf.noContents": "This document carries no table of contents.",
    "pdf.pages": ".input {$count :number}\n.match $count\none {{1 page}}\n*   {{{$count} pages}}",
    "pdf.onePage": "1 page",
    "pdf.page": "Page {$number}",
    "pdf.pageOf": "Page {$number} of {$total}",
    "pdf.search.label": "Search in document",
    "pdf.search.none": "No page contains that.",
    "pdf.search.noDocument": "There is no document open to search.",
    "pdf.search.lockLost":
      "The document is in an unknown state, so it could not be searched. Open the file again.",
    "pdf.search.failed": "That search did not run. The document is still open.",
    "pdf.search.unsearchable":
      "{$count} pages could not be read, so anything on them is missing from these results.",
    "pdf.nothingOpen": "No document is open. Open a PDF from Files, or pass one on the command line.",
    "pdf.failed": "This document could not be opened.",
    "pdf.notFound": "There is no file at {$path}.",
    "pdf.noPermission": "This account may not read {$path}.",
    "pdf.unreadableFile": "{$path} could not be read.",
    "pdf.notAPdf": "This file is not a PDF this reader can read.",
    "pdf.noPages": "This PDF contains no pages.",
    "pdf.launchUnknown": "The reader could not find out which document it was asked to open.",
    "pdf.locked": "This document is locked with a password. This reader cannot ask for one yet.",
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
    "pdf.pageFailed": "This page could not be drawn.",
    "pdf.pageLockLost":
      "The document is in an unknown state, so this page could not be drawn. Open the file again.",
    "pdf.textFace": "This machine has nothing installed that can draw a page, so the document is shown as its text, without its layout.",
    "pdf.pageNoText": "This page carries no text. It may be a scanned image.",
    "pdf.pageTextInstead": "This page could not be drawn, so its text is shown here, without its layout.",
  },
  de: {
    "pdf.app.title": "Dokument",
    "pdf.contents": "Inhalt",
    "pdf.pageFlow": "Die Seiten des Dokuments",
    "pdf.noContents": "Dieses Dokument hat kein Inhaltsverzeichnis.",
    "pdf.pages": ".input {$count :number}\n.match $count\none {{1 Seite}}\n*   {{{$count} Seiten}}",
    "pdf.onePage": "1 Seite",
    "pdf.page": "Seite {$number}",
    "pdf.pageOf": "Seite {$number} von {$total}",
    "pdf.search.label": "Im Dokument suchen",
    "pdf.search.none": "Keine Seite enthält das.",
    "pdf.search.noDocument": "Es ist kein Dokument offen, in dem gesucht werden könnte.",
    "pdf.search.lockLost":
      "Das Dokument ist in einem unbekannten Zustand und konnte nicht durchsucht werden. Öffne die Datei erneut.",
    "pdf.search.failed": "Diese Suche wurde nicht ausgeführt. Das Dokument ist weiterhin offen.",
    "pdf.search.unsearchable":
      "{$count} Seiten konnten nicht gelesen werden, deshalb fehlt alles darauf in diesen Ergebnissen.",
    "pdf.nothingOpen":
      "Es ist kein Dokument geöffnet. Öffne ein PDF aus Dateien oder übergib eins auf der Kommandozeile.",
    "pdf.failed": "Dieses Dokument konnte nicht geöffnet werden.",
    "pdf.notFound": "Unter {$path} liegt keine Datei.",
    "pdf.noPermission": "Dieses Konto darf {$path} nicht lesen.",
    "pdf.unreadableFile": "{$path} konnte nicht gelesen werden.",
    "pdf.notAPdf": "Diese Datei ist kein PDF, das dieser Leser lesen kann.",
    "pdf.noPages": "Dieses PDF enthält keine Seiten.",
    "pdf.launchUnknown": "Der Leser konnte nicht herausfinden, welches Dokument er öffnen sollte.",
    "pdf.locked": "Dieses Dokument ist mit einem Passwort gesperrt. Dieser Leser kann noch keins abfragen.",
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
    // "Leser", like the three keys above. This one kept "Reader" when the others
    // were corrected - the same English word in the same app, in the sentence
    // nobody reaches on a real machine and therefore nobody re-read.
    "pdf.hostAbsent": "Der Leser liest Dokumente über seinen eigenen Host, der hier nicht läuft.",
    "pdf.pageFailed": "Diese Seite konnte nicht gezeichnet werden.",
    "pdf.pageLockLost":
      "Das Dokument ist in einem unbekannten Zustand, deshalb konnte diese Seite nicht gezeichnet werden. Öffne die Datei erneut.",
    "pdf.textFace": "Auf diesem Rechner ist nichts installiert, das eine Seite zeichnen kann, deshalb wird das Dokument als sein Text gezeigt, ohne sein Layout.",
    "pdf.pageNoText": "Diese Seite trägt keinen Text. Sie ist womöglich ein gescanntes Bild.",
    "pdf.pageTextInstead": "Diese Seite konnte nicht gezeichnet werden, deshalb steht hier ihr Text, ohne sein Layout.",
  },
};

export const t = createTranslator(messages);

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
    "pdf.search.label": "Search in document",
    "pdf.search.none": "No page contains that.",
    "pdf.search.unsearchable":
      "{$count} pages could not be read, so anything on them is missing from these results.",
    "pdf.nothingOpen": "No document is open. Open a PDF from the file manager, or pass one on the command line.",
    "pdf.failed": "Could not open this document: {$reason}",
    "pdf.hostAbsent": "The reader reads documents through its own host, which is not running here.",
    "pdf.pageFailed": "This page could not be drawn: {$reason}",
  },
  de: {
    "pdf.app.title": "Dokument",
    "pdf.contents": "Inhalt",
    "pdf.noContents": "Dieses Dokument hat kein Inhaltsverzeichnis.",
    "pdf.pages": "{$count} Seiten",
    "pdf.onePage": "1 Seite",
    "pdf.page": "Seite {$number}",
    "pdf.search.label": "Im Dokument suchen",
    "pdf.search.none": "Keine Seite enthält das.",
    "pdf.search.unsearchable":
      "{$count} Seiten konnten nicht gelesen werden, deshalb fehlt alles darauf in diesen Ergebnissen.",
    "pdf.nothingOpen":
      "Es ist kein Dokument geöffnet. Öffne ein PDF aus der Dateiverwaltung oder übergib eins auf der Kommandozeile.",
    "pdf.failed": "Dieses Dokument konnte nicht geöffnet werden: {$reason}",
    "pdf.hostAbsent": "Der Reader liest Dokumente über seinen eigenen Host, der hier nicht läuft.",
    "pdf.pageFailed": "Diese Seite konnte nicht gezeichnet werden: {$reason}",
  },
};

export const t = createTranslator(messages);

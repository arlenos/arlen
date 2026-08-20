/// The screenshot app's message catalog, authored in MessageFormat 2.0. English is
/// the source of truth; German proves the reactive locale switch. Same template as the
/// settings, desktop-shell and viewers catalogs (I18N-R4).
///
/// The shortcut hints in the tooltips keep their key names in the message text: a
/// translator moves the words around them, but `Ctrl+Z` is a key combination the user
/// presses and reads the same everywhere.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";

/// Re-exported so the app and a future language switcher drive the same shared store
/// instance the catalog is bound to.
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: {
    "s.app.title": "Screenshot",
    "s.screenCapture": "Screen capture",
    "s.captureActions": "Capture actions",
    "s.annotate": "Annotate",
    "s.copy": "Copy",
    "s.save": "Save",
    "s.saveFailed": "That screenshot was not saved.",
    "s.captureUnavailable": "Could not take a picture of your screen, so there is nothing here to annotate or save.",
    "s.sampleShot": "This is an example picture, not your screen.",
    "s.source": "Showing",
    "s.source.screen": "Screen {$name}",
    "s.source.screenSized": "Screen {$name} ({$w}x{$h})",
    "s.source.window": "Window: {$title}",
    "s.source.windowUntitled": "Window with no title",
    "s.copyFailed": "That screenshot was not copied.",
    "s.dismiss": "Dismiss",
    "s.typePlaceholder": "Type…",
    "s.undo": "Undo",
    "s.undoHint": "Undo (Ctrl+Z)",
    "s.redo": "Redo",
    "s.redoHint": "Redo (Ctrl+Shift+Z)",
    "s.copyHint": "Copy (Enter)",
    "s.savedToPictures": "Saved to Pictures.",
    "s.tool.select": "Select",
    "s.tool.crop": "Crop",
    "s.tool.arrow": "Arrow",
    "s.tool.box": "Box",
    "s.tool.ellipse": "Ellipse",
    "s.tool.text": "Text",
    "s.tool.pen": "Pen",
    "s.tool.highlight": "Highlighter",
    "s.tool.blur": "Blur / redact",
    "s.tool.number": "Step",
    "s.size.thin": "Thin",
    "s.size.medium": "Medium",
    "s.size.thick": "Thick",
  },
  de: {
    "s.app.title": "Bildschirmfoto",

    "s.screenCapture": "Bildschirmaufnahme",
    "s.captureActions": "Aktionen zur Aufnahme",
    "s.annotate": "Beschriften",
    "s.copy": "Kopieren",
    "s.save": "Speichern",
    "s.saveFailed": "Dieser Screenshot wurde nicht gespeichert.",
    "s.captureUnavailable": "Von deinem Bildschirm konnte kein Bild gemacht werden, deshalb gibt es hier nichts zum Bearbeiten oder Speichern.",
    "s.sampleShot": "Das ist ein Beispielbild, nicht dein Bildschirm.",
    "s.source": "Gezeigt wird",
    "s.source.screen": "Bildschirm {$name}",
    "s.source.screenSized": "Bildschirm {$name} ({$w}x{$h})",
    "s.source.window": "Fenster: {$title}",
    "s.source.windowUntitled": "Fenster ohne Titel",
    "s.copyFailed": "Dieser Screenshot wurde nicht kopiert.",
    "s.dismiss": "Verwerfen",
    "s.typePlaceholder": "Tippen…",
    "s.undo": "Rückgängig",
    "s.undoHint": "Rückgängig (Strg+Z)",
    "s.redo": "Wiederholen",
    "s.redoHint": "Wiederholen (Strg+Umschalt+Z)",
    "s.copyHint": "Kopieren (Enter)",
    "s.savedToPictures": "In Bilder gespeichert.",
    "s.tool.select": "Auswählen",
    "s.tool.crop": "Zuschneiden",
    "s.tool.arrow": "Pfeil",
    "s.tool.box": "Rechteck",
    "s.tool.ellipse": "Ellipse",
    "s.tool.text": "Text",
    "s.tool.pen": "Stift",
    "s.tool.highlight": "Textmarker",
    "s.tool.blur": "Unkenntlich machen",
    "s.tool.number": "Schritt",
    "s.size.thin": "Dünn",
    "s.size.medium": "Mittel",
    "s.size.thick": "Dick",
  },
};

/// The bound translator: `$t("s.key", params?)`, reactive to `locale`.
export const t = createTranslator(messages);

/// The Meetings message catalog, authored in MessageFormat 2.0. English is the source
/// of truth; German proves the reactive locale switch. The full extraction sweep across
/// every app is a separate job (I18N-R4); this follows the system-monitor template.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";

/// Re-exported so the app's locale control (and tests) drive the same shared store
/// instance the catalog is bound to.
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: {
    "mt.title": "Meetings",
    "mt.start": "Start a meeting",
    "mt.empty": "No meetings yet. Start one to capture it on this device.",
    "mt.foot": "Captured and kept on this device.",
    "mt.open": "Open in editor",
    "mt.yourNotes": "Your notes",
    "mt.summary": "Summary",
    "mt.summary.from": "from the recording",
    "mt.grounded": "Every line is drawn from the transcript. Read it on the right to check.",
    "mt.actionItems": "Action items",
    "mt.actionItems.none": "None captured.",
    "mt.add": "Add",
    "mt.add.title": "Add to your calendar (asks first)",
    "mt.recording": "Recording",
    "mt.stop": "Stop",
    "mt.notes.placeholder": "Jot what matters. The AI fills the rest in from the recording.",
    "mt.transcript": "Transcript",
    "mt.speaker": "Speaker {$n}",
    "mt.speaker.generic": "Speaker",
  },
  de: {
    "mt.title": "Besprechungen",
    "mt.start": "Besprechung starten",
    "mt.empty": "Noch keine Besprechungen. Starte eine, um sie auf diesem Gerät aufzunehmen.",
    "mt.foot": "Auf diesem Gerät aufgenommen und gespeichert.",
    "mt.open": "Im Editor öffnen",
    "mt.yourNotes": "Deine Notizen",
    "mt.summary": "Zusammenfassung",
    "mt.summary.from": "aus der Aufnahme",
    "mt.grounded": "Jede Zeile stammt aus dem Transkript. Prüfe es rechts.",
    "mt.actionItems": "Aufgaben",
    "mt.actionItems.none": "Keine erfasst.",
    "mt.add": "Hinzufügen",
    "mt.add.title": "Zu deinem Kalender hinzufügen (fragt erst)",
    "mt.recording": "Nimmt auf",
    "mt.stop": "Stopp",
    "mt.notes.placeholder": "Notiere das Wichtige. Die KI ergänzt den Rest aus der Aufnahme.",
    "mt.transcript": "Transkript",
    "mt.speaker": "Sprecher {$n}",
    "mt.speaker.generic": "Sprecher",
  },
};

/// The bound translator: `$t("mt.key", params?)`, reactive to `locale`.
export const t = createTranslator(messages);

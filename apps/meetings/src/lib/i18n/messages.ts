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
    "mt.newMeeting": "New meeting",
    "mt.pickHint": "Pick a meeting from the list or start a new one.",
    "mt.empty": "No meetings yet. Start one to capture it on this device.",
    "mt.open": "Open in editor",
    "mt.sample.list": "Example meetings - not your real history.",
    "mt.sample": "Example note - not a real meeting. The participants and quotes are made up.",
    "mt.editFailed": "That edit was not saved, so what you see is the last saved version.",
    "mt.yourNotes": "Your notes",
    "mt.notes.merged": "Notes",
    "mt.notes.edit": "Edit notes",
    "mt.notes.save": "Save",
    "mt.notes.cancel": "Cancel",
    "mt.notes.also": "Also from the meeting",
    "mt.grounded": "Your lines stay yours; the tinted lines are the AI's, drawn from the transcript. Click one to check it.",
    "mt.actionItems": "Action items",
    "mt.actionItems.none": "None captured.",
    "mt.owner": "Owner",
    "mt.owner.set": "Set owner",
    "mt.owner.edit": "Change who this belongs to",
    "mt.back": "Back",
    "mt.recording": "Recording",
    "mt.stop": "Stop",
    "mt.consent": "Recording locally, nothing joins the call.",
    "mt.transcribe": "Transcribe",
    "mt.transcribe.off": "Transcription is off. The recording continues; no text is produced.",
    "mt.notes.placeholder": "Jot what matters. The AI fills the rest in from the recording.",
    "mt.transcript": "Transcript",
    "mt.speaker": "Speaker {$n}",
    "mt.speaker.generic": "Speaker",
    "mt.speaker.rename": "Rename this speaker",
    "mt.speaker.unsure": "unsure",
  },
  de: {
    "mt.title": "Besprechungen",
    "mt.start": "Besprechung starten",
    "mt.newMeeting": "Neue Besprechung",
    "mt.pickHint": "Wähle eine Besprechung aus der Liste oder starte eine neue.",
    "mt.empty": "Noch keine Besprechungen. Starte eine, um sie auf diesem Gerät aufzunehmen.",
    "mt.open": "Im Editor öffnen",
    "mt.sample.list": "Beispiel-Meetings - nicht dein echter Verlauf.",
    "mt.sample": "Beispielnotiz - kein echtes Meeting. Teilnehmende und Zitate sind erfunden.",
    "mt.editFailed": "Die Änderung wurde nicht gespeichert, auf dem Bildschirm steht wieder der zuletzt gespeicherte Stand.",
    "mt.yourNotes": "Deine Notizen",
    "mt.notes.merged": "Notizen",
    "mt.notes.edit": "Notizen bearbeiten",
    "mt.notes.save": "Speichern",
    "mt.notes.cancel": "Abbrechen",
    "mt.notes.also": "Außerdem aus dem Meeting",
    "mt.grounded": "Deine Zeilen bleiben deine; die getönten sind von der KI, aus dem Transkript. Klick eine an, um sie zu prüfen.",
    "mt.actionItems": "Aufgaben",
    "mt.actionItems.none": "Keine erfasst.",
    "mt.owner": "Zuständig",
    "mt.owner.set": "Zuständigkeit setzen",
    "mt.owner.edit": "Ändern, wem das gehört",
    "mt.back": "Zurück",
    "mt.recording": "Nimmt auf",
    "mt.stop": "Stopp",
    "mt.consent": "Nimmt lokal auf, nichts tritt dem Call bei.",
    "mt.transcribe": "Transkribieren",
    "mt.transcribe.off": "Transkription ist aus. Die Aufnahme läuft weiter; es entsteht kein Text.",
    "mt.notes.placeholder": "Notiere das Wichtige. Die KI ergänzt den Rest aus der Aufnahme.",
    "mt.transcript": "Transkript",
    "mt.speaker": "Sprecher {$n}",
    "mt.speaker.generic": "Sprecher",
    "mt.speaker.rename": "Diesen Sprecher umbenennen",
    "mt.speaker.unsure": "unsicher",
  },
};

/// The bound translator: `$t("mt.key", params?)`, reactive to `locale`.
export const t = createTranslator(messages);

/// The calendar app's message catalog (MF2, en + de). Keys are prefixed `cal.`
/// per the per-app convention. Dates and times format through Intl off the shared
/// locale store, never through catalog strings.
///
/// Born translatable: every sentence a person reads is a key here, in both
/// languages, from the first commit. Retrofitting this is the expensive way, and
/// rendering the German build is what catches the sentences that only sound right
/// in English.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: {
    "cal.app.title": "Calendar",
    "cal.agenda": "Agenda",
    "cal.allDay": "All day",
    "cal.utc": "UTC",
    "cal.repeats": "Repeats",
    "cal.repeatsUnexpanded": "This event repeats, and its rule is one the calendar cannot work out yet, so only this one is shown.",
    "cal.repeatsShown": "One of a repeating series.",
    "cal.empty": "The calendar files in {$dir} have no events in them.",
    "cal.noFiles": "No calendar files yet. Put .ics files in {$dir} and they show up here.",
    "cal.unreadable": "{$count} of your calendar files could not be read, so events in them are missing.",
    "cal.hostAbsent": "The calendar reads your files through its own host, which is not running here.",
    "cal.serviceDown": "Showing your files directly: the calendar service is not running, so no reminders are being set for these events.",
    "cal.failed": "Could not read your calendar files: {$reason}",
  },
  de: {
    "cal.app.title": "Kalender",
    "cal.agenda": "Termine",
    "cal.allDay": "Ganztägig",
    "cal.utc": "UTC",
    "cal.repeats": "Wiederholt sich",
    "cal.repeatsUnexpanded": "Dieser Termin wiederholt sich, und seine Regel kann der Kalender noch nicht ausrechnen, deshalb steht hier nur dieser eine.",
    "cal.repeatsShown": "Einer aus einer Wiederholungsreihe.",
    "cal.empty": "Die Kalenderdateien in {$dir} enthalten keine Termine.",
    "cal.noFiles": "Noch keine Kalenderdateien. Lege .ics-Dateien in {$dir} ab, dann erscheinen sie hier.",
    "cal.unreadable": "{$count} deiner Kalenderdateien konnten nicht gelesen werden, die Termine darin fehlen also.",
    "cal.hostAbsent": "Der Kalender liest deine Dateien über seinen eigenen Host, der hier nicht läuft.",
    "cal.serviceDown": "Deine Dateien werden direkt gelesen: der Kalenderdienst läuft nicht, deshalb werden für diese Termine keine Erinnerungen gestellt.",
    "cal.failed": "Deine Kalenderdateien konnten nicht gelesen werden: {$reason}",
  },
};

/// The calendar app's reactive translator.
export const t = createTranslator(messages);

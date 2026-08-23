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
    "cal.today": "Today",
    "cal.everyDaily": ".input {$n :number}\n.match $n\none {{Every day}}\n* {{Every {$n} days}}",
    "cal.everyWeekly": ".input {$n :number}\n.match $n\none {{Every week}}\n* {{Every {$n} weeks}}",
    "cal.everyMonthly": ".input {$n :number}\n.match $n\none {{Every month}}\n* {{Every {$n} months}}",
    "cal.everyYearly": ".input {$n :number}\n.match $n\none {{Every year}}\n* {{Every {$n} years}}",
    "cal.onDays": "{$every} on {$days}",
    "cal.dayMon": "Mon",
    "cal.dayTue": "Tue",
    "cal.dayWed": "Wed",
    "cal.dayThu": "Thu",
    "cal.dayFri": "Fri",
    "cal.daySat": "Sat",
    "cal.daySun": "Sun",
    "cal.repeatsUnexpanded": "This event repeats, and its rule is one the calendar cannot work out yet, so only this one is shown.",
    "cal.repeatsShown": "One of a repeating series.",
    // The SHORT form of the caveat above, rendered in the row rather than hidden
    // in a tooltip. A title attribute is not a statement to a reader: it needs a
    // pointer that hovers, so a person reading the agenda at a glance, or driving
    // it from the keyboard, is told nothing at all - and what they are not told
    // here is that the later dates of this series are missing from their agenda.
    "cal.onlyThisOne": "only this date",
    "cal.empty": "The calendar files in {$dir} have no events in them.",
    "cal.noFiles": "No calendar files yet. Put .ics files in {$dir} and they show up here.",
    "cal.keep": "Keep this calendar",
    "cal.unreadable": "{$count} of your calendar files could not be read, so events in them are missing.",
    "cal.hostAbsent": "The calendar reads your files through its own host, which is not running here.",
    "cal.serviceDown": "Showing your files directly: the calendar service is not running, so no reminders are being set for these events.",
    // Named causes rather than whatever the layer below said. The five keep
    // failures used to reach the window as English sentences with no catalogue
    // around them at all.
    "cal.failed.noHome": "This session has no home directory, so there is no calendar folder to read.",
    "cal.failed.unreadable": "Your calendar folder could not be read, so nothing is shown: {$why}",
    "cal.failed.other": "Could not read your calendar files: {$reason}",
    "cal.keep.notAFile": "That is not a file, so there is nothing to keep.",
    "cal.keep.noHome": "This session has no home directory, so there is nowhere to keep it.",
    "cal.keep.cannotMakeDir": "The calendar folder could not be made, so it was not kept: {$why}",
    "cal.keep.alreadyKept": "A calendar called {$name} is already kept. Rename one of them and try again.",
    "cal.keep.copyFailed": "It could not be copied into the calendar folder: {$why}",
  },
  de: {
    "cal.app.title": "Kalender",
    "cal.agenda": "Termine",
    "cal.allDay": "Ganztägig",
    "cal.utc": "UTC",
    "cal.repeats": "Wiederholt sich",
    "cal.today": "Heute",
    "cal.everyDaily": ".input {$n :number}\n.match $n\none {{Jeden Tag}}\n* {{Alle {$n} Tage}}",
    "cal.everyWeekly": ".input {$n :number}\n.match $n\none {{Jede Woche}}\n* {{Alle {$n} Wochen}}",
    "cal.everyMonthly": ".input {$n :number}\n.match $n\none {{Jeden Monat}}\n* {{Alle {$n} Monate}}",
    "cal.everyYearly": ".input {$n :number}\n.match $n\none {{Jedes Jahr}}\n* {{Alle {$n} Jahre}}",
    "cal.onDays": "{$every} am {$days}",
    "cal.dayMon": "Mo",
    "cal.dayTue": "Di",
    "cal.dayWed": "Mi",
    "cal.dayThu": "Do",
    "cal.dayFri": "Fr",
    "cal.daySat": "Sa",
    "cal.daySun": "So",
    "cal.repeatsUnexpanded": "Dieser Termin wiederholt sich, und seine Regel kann der Kalender noch nicht ausrechnen, deshalb steht hier nur dieser eine.",
    "cal.repeatsShown": "Einer aus einer Wiederholungsreihe.",
    "cal.onlyThisOne": "nur dieses Datum",
    "cal.empty": "Die Kalenderdateien in {$dir} enthalten keine Termine.",
    "cal.noFiles": "Noch keine Kalenderdateien. Lege .ics-Dateien in {$dir} ab, dann erscheinen sie hier.",
    "cal.keep": "Diesen Kalender behalten",
    "cal.unreadable": "{$count} deiner Kalenderdateien konnten nicht gelesen werden, die Termine darin fehlen also.",
    "cal.hostAbsent": "Der Kalender liest deine Dateien über seinen eigenen Host, der hier nicht läuft.",
    "cal.serviceDown": "Deine Dateien werden direkt gelesen: der Kalenderdienst läuft nicht, deshalb werden für diese Termine keine Erinnerungen gestellt.",
    "cal.failed.noHome": "Diese Sitzung hat kein Home-Verzeichnis, es gibt also keinen Kalenderordner zu lesen.",
    "cal.failed.unreadable": "Dein Kalenderordner konnte nicht gelesen werden, deshalb wird nichts gezeigt: {$why}",
    "cal.failed.other": "Deine Kalenderdateien konnten nicht gelesen werden: {$reason}",
    "cal.keep.notAFile": "Das ist keine Datei, es gibt also nichts zu behalten.",
    "cal.keep.noHome": "Diese Sitzung hat kein Home-Verzeichnis, es gibt also keinen Ort dafür.",
    "cal.keep.cannotMakeDir": "Der Kalenderordner konnte nicht angelegt werden, deshalb wurde nichts behalten: {$why}",
    "cal.keep.alreadyKept": "Ein Kalender namens {$name} wird bereits behalten. Benenne einen von beiden um und versuche es erneut.",
    "cal.keep.copyFailed": "Er konnte nicht in den Kalenderordner kopiert werden: {$why}",
  },
};

/// The calendar app's reactive translator.
export const t = createTranslator(messages);

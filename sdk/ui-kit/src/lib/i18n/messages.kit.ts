// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

/// The kit's own message catalog, and the translator bound to it.
///
/// Shared components cannot reach an app's catalog - they are compiled into every
/// app and none of them owns the strings. The three ways out are: pass every label
/// as a prop (a window-control would take three, a file browser a dozen, and each
/// app would repeat them), copy the kit's strings into each app's catalog (the same
/// fact stored once per app, drifting), or let the kit own its strings. It owns
/// them.
///
/// `kt` is a separate translator from an app's `t`, but not a separate locale: both
/// derive from the one `locale` store this module exports, so switching the language
/// re-renders kit chrome and app content together. Ids are prefixed `k.` so a kit
/// string is recognisable at the call site and cannot collide with an app's.

import { createTranslator, type Catalogs } from "./index";

const kitMessages: Catalogs = {
  en: {
    "k.window.minimize": "Minimize",
    "k.window.maximize": "Maximize",
    "k.window.restore": "Restore",
    "k.window.close": "Close",
    "k.sidebar.toggle": "Toggle Sidebar",
    "k.chip.add": "Add…",
    "k.confirm.confirm": "Confirm",
    "k.confirm.working": "Working…",
    "k.picker.chooseFolder": "Choose folder",
    "k.select.none": "None",
    "k.action.cancel": "Cancel",
    "k.action.remove": "Remove",
    "k.action.close": "Close",
    "k.about.title": "About {$app}",
    "k.about.build": "Arlen OS · {$version}",
    "k.media.previous": "Previous",
    "k.media.next": "Next",
    "k.media.play": "Play",
    "k.media.pause": "Pause",
    "k.number.decrease": "Decrease",
    "k.number.increase": "Increase",
    "k.number.decreaseNamed": "Decrease {$field}",
    "k.number.increaseNamed": "Increase {$field}",
    "k.colour.hue": "Hue",
    "k.colour.hex": "Hex colour",
    "k.browser.path": "Path",
    "k.browser.files": "Files",
    "k.browser.newName": "New name",
    "k.browser.columns": "Folder columns",
    "k.browser.empty": "Empty",
    "k.browser.offline": "{$place} (not connected)",
    "k.browser.unpin": "Unpin {$place}",
    "k.toast.position": "Toast position",
    "k.days.group": "Days of week",
    "k.console.running": "running",
    "k.console.stillRunning": "Still running",
    "k.console.exit": "exit {$code}",
  },
  de: {
    "k.window.minimize": "Minimieren",
    "k.window.maximize": "Maximieren",
    "k.window.restore": "Wiederherstellen",
    "k.window.close": "Schließen",
    "k.sidebar.toggle": "Seitenleiste umschalten",
    "k.chip.add": "Hinzufügen …",
    "k.confirm.confirm": "Bestätigen",
    "k.confirm.working": "Wird ausgeführt …",
    "k.picker.chooseFolder": "Ordner wählen",
    "k.select.none": "Keine",
    "k.action.cancel": "Abbrechen",
    "k.action.remove": "Entfernen",
    "k.action.close": "Schließen",
    "k.about.title": "Über {$app}",
    "k.about.build": "Arlen OS · {$version}",
    "k.media.previous": "Zurück",
    "k.media.next": "Weiter",
    "k.media.play": "Abspielen",
    "k.media.pause": "Pause",
    "k.number.decrease": "Verringern",
    "k.number.increase": "Erhöhen",
    "k.number.decreaseNamed": "{$field} verringern",
    "k.number.increaseNamed": "{$field} erhöhen",
    "k.colour.hue": "Farbton",
    "k.colour.hex": "Hex-Farbe",
    "k.browser.path": "Pfad",
    "k.browser.files": "Dateien",
    "k.browser.newName": "Neuer Name",
    "k.browser.columns": "Ordnerspalten",
    "k.browser.empty": "Leer",
    "k.browser.offline": "{$place} (nicht verbunden)",
    "k.browser.unpin": "{$place} lösen",
    "k.toast.position": "Position der Hinweise",
    "k.days.group": "Wochentage",
    "k.console.running": "läuft",
    "k.console.stillRunning": "Läuft noch",
    "k.console.exit": "Exit-Code {$code}",
  },
};

/// The kit's translator. Use it in kit components exactly as an app uses its own:
/// `{$kt("k.window.close")}`.
export const kt = createTranslator(kitMessages);

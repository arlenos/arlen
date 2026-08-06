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
  },
  de: {
    "k.window.minimize": "Minimieren",
    "k.window.maximize": "Maximieren",
    "k.window.restore": "Wiederherstellen",
    "k.window.close": "Schließen",
  },
};

/// The kit's translator. Use it in kit components exactly as an app uses its own:
/// `{$kt("k.window.close")}`.
export const kt = createTranslator(kitMessages);

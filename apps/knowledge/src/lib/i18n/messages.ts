/// The Knowledge app message catalog, authored in MessageFormat 2.0. English is the
/// source of truth; German proves the reactive locale switch. Follows the meetings /
/// system-monitor template. Key prefix: `k.`.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";

/// Re-exported so the app's locale control (and tests) drive the same shared store.
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: {
    "k.title": "Knowledge",
    "k.sample": "Example data - not your real graph yet.",
    "k.section.explore": "Explore",
    "k.section.authority": "Authority",
    "k.place.timeline": "Timeline",
    "k.place.projects": "Projects",
    "k.place.searches": "Searches",
    "k.place.library": "Library",
    "k.place.capsules": "Capsules",
    "k.place.capabilities": "Capabilities",
    "k.caps.opens": "Opens in Settings",
    "k.empty.timeline": "Nothing recorded here yet.",
    "k.empty.projects": "No projects detected yet.",
    "k.empty.searches": "No saved searches yet.",
    "k.empty.library": "No sources bridged in yet.",
    "k.empty.capsules": "No shared slices yet.",
    "k.detail.title": "Details",
    "k.detail.none": "Select something to see its details.",
    "k.detail.when": "When",
    "k.detail.more": "Relationships and provenance land as the Knowledge app grows.",
    "k.close": "Close",
  },
  de: {
    "k.title": "Wissen",
    "k.sample": "Beispieldaten - noch nicht dein echter Graph.",
    "k.section.explore": "Erkunden",
    "k.section.authority": "Zugriff",
    "k.place.timeline": "Verlauf",
    "k.place.projects": "Projekte",
    "k.place.searches": "Suchen",
    "k.place.library": "Bibliothek",
    "k.place.capsules": "Kapseln",
    "k.place.capabilities": "Berechtigungen",
    "k.caps.opens": "Öffnet in Einstellungen",
    "k.empty.timeline": "Hier ist noch nichts aufgezeichnet.",
    "k.empty.projects": "Noch keine Projekte erkannt.",
    "k.empty.searches": "Noch keine gespeicherten Suchen.",
    "k.empty.library": "Noch keine Quellen eingebunden.",
    "k.empty.capsules": "Noch keine geteilten Ausschnitte.",
    "k.detail.title": "Details",
    "k.detail.none": "Wähle etwas, um die Details zu sehen.",
    "k.detail.when": "Wann",
    "k.detail.more": "Beziehungen und Herkunft kommen, während die Wissens-App wächst.",
    "k.close": "Schließen",
  },
};

/// The reactive translator bound to the shared locale store.
export const t = createTranslator(messages);

/// The text-editor message catalog, authored in MessageFormat 2.0. English is the
/// source of truth; German proves the reactive locale switch. Follows the
/// system-monitor + meetings template (I18N-R4). File content and KG data are data,
/// not chrome, so they stay out of the catalog.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";

/// Re-exported so the app (and tests) drive the same shared store instance the
/// catalog is bound to.
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: {
    "te.openFile": "Open file",
    "te.lineNumbers.toggle": "Toggle line numbers",
    "te.lineNumbers": "Line numbers",
    "te.focus": "Focus",
    "te.asOf.aria": "Show the file as of",
    "te.asOf.now": "Now",
    "te.asOf.1d": "1 day ago",
    "te.asOf.1w": "1 week ago",
    "te.asOf.1m": "1 month ago",
    "te.lens.toggle": "Toggle the lens",
    "te.lens.provenance": "Where it came from",
    "te.lens.related": "Related",
    "te.lens.related.empty": "Nothing references this file yet.",
    "te.lens.project": "Project",
    "te.lens.project.partOf": "Part of {$name}",
    "te.lens.sample": "Example context - not this file's real graph neighbourhood.",
    "te.review.undone": "Undone",
    "te.review.rejected": "Rejected",
    "te.review.held": "Needs your confirmation",
    "te.review.appliedAuto": "Applied on its own",
    "te.review.applied": "Applied",
    "te.review.dismiss": "Dismiss",
    "te.review.youAsked": "You asked: {$prompt}",
    "te.review.reject": "Reject",
    "te.review.accept": "Accept",
    "te.review.undo": "Undo",
    "te.review.foot": "Every change is logged, and you can undo any of it. Turn the assistant off in Settings.",
  },
  de: {
    "te.openFile": "Datei öffnen",
    "te.lineNumbers.toggle": "Zeilennummern umschalten",
    "te.lineNumbers": "Zeilennummern",
    "te.focus": "Fokus",
    "te.asOf.aria": "Datei anzeigen zum Stand",
    "te.asOf.now": "Jetzt",
    "te.asOf.1d": "vor 1 Tag",
    "te.asOf.1w": "vor 1 Woche",
    "te.asOf.1m": "vor 1 Monat",
    "te.lens.toggle": "Lens umschalten",
    "te.lens.provenance": "Woher es kommt",
    "te.lens.related": "Verwandt",
    "te.lens.related.empty": "Nichts verweist bisher auf diese Datei.",
    "te.lens.project": "Projekt",
    "te.lens.project.partOf": "Teil von {$name}",
    "te.lens.sample": "Beispielkontext - nicht die echte Graph-Nachbarschaft dieser Datei.",
    "te.review.undone": "Rückgängig gemacht",
    "te.review.rejected": "Abgelehnt",
    "te.review.held": "Braucht deine Bestätigung",
    "te.review.appliedAuto": "Selbstständig angewendet",
    "te.review.applied": "Angewendet",
    "te.review.dismiss": "Schließen",
    "te.review.youAsked": "Du hast gefragt: {$prompt}",
    "te.review.reject": "Ablehnen",
    "te.review.accept": "Übernehmen",
    "te.review.undo": "Rückgängig",
    "te.review.foot": "Jede Änderung wird protokolliert, und du kannst alles rückgängig machen. Schalte den Assistenten in den Einstellungen aus.",
  },
};

/// The bound translator: `$t("te.key", params?)`, reactive to `locale`.
export const t = createTranslator(messages);

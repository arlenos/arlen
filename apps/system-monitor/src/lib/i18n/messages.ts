/// The task-manager message catalog, authored in MessageFormat 2.0. The English map
/// is the source of truth; German proves the reactive locale switch + MF2 plurals.
/// The full extraction sweep across every app is a separate job (I18N-R4); this app
/// is the reference template for the pattern.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";

/// Re-exported so the app's locale control (and tests) drive the same shared store
/// instance the catalog is bound to.
export { locale, dir } from "@arlen/ui-kit/i18n";

const PROCESS_COUNT_EN = ".input {$count :number}\n.match $count\none {{{$count} process}}\n*   {{{$count} processes}}";
const PROCESS_COUNT_DE = ".input {$count :number}\n.match $count\none {{{$count} Prozess}}\n*   {{{$count} Prozesse}}";

const messages: Catalogs = {
  en: {
    "tm.title": "Task manager",
    "tm.tab.processes": "Processes",
    "tm.tab.performance": "Performance",
    "tm.filter.placeholder": "Filter",
    "tm.filter.aria": "Filter processes",
    "tm.toggle.grouped": "Grouped",
    "tm.toggle.all": "All processes",
    "tm.toggle.toGrouped": "Group by app",
    "tm.toggle.toAll": "Show every process",
    "tm.group.app": "Apps",
    "tm.group.background": "Background",
    "tm.group.system": "System",
    "tm.col.name": "Name",
    "tm.col.status": "Status",
    "tm.col.access": "Access",
    "tm.col.cpu": "CPU",
    "tm.col.memory": "Memory",
    "tm.col.disk": "Disk",
    "tm.col.network": "Network",
    "tm.status.running": "Running",
    "tm.status.notResponding": "Not responding",
    "tm.status.suspended": "Suspended",
    "tm.tag.limited": "Limited",
    "tm.row.expand": "Expand",
    "tm.grid.label": PROCESS_COUNT_EN,
    "tm.menu.details": "Show details",
    "tm.menu.resume": "Resume",
    "tm.menu.pause": "Pause",
    "tm.menu.unlimit": "Remove limit",
    "tm.menu.limit": "Limit",
    "tm.menu.stop": "Stop",
    "tm.menu.forceQuit": "Force quit",
  },
  de: {
    "tm.title": "Task-Manager",
    "tm.tab.processes": "Prozesse",
    "tm.tab.performance": "Leistung",
    "tm.filter.placeholder": "Filtern",
    "tm.filter.aria": "Prozesse filtern",
    "tm.toggle.grouped": "Gruppiert",
    "tm.toggle.all": "Alle Prozesse",
    "tm.toggle.toGrouped": "Nach App gruppieren",
    "tm.toggle.toAll": "Jeden Prozess zeigen",
    "tm.group.app": "Apps",
    "tm.group.background": "Hintergrund",
    "tm.group.system": "System",
    "tm.col.name": "Name",
    "tm.col.status": "Status",
    "tm.col.access": "Zugriff",
    "tm.col.cpu": "CPU",
    "tm.col.memory": "Speicher",
    "tm.col.disk": "Datenträger",
    "tm.col.network": "Netzwerk",
    "tm.status.running": "Läuft",
    "tm.status.notResponding": "Reagiert nicht",
    "tm.status.suspended": "Angehalten",
    "tm.tag.limited": "Begrenzt",
    "tm.row.expand": "Ausklappen",
    "tm.grid.label": PROCESS_COUNT_DE,
    "tm.menu.details": "Details anzeigen",
    "tm.menu.resume": "Fortsetzen",
    "tm.menu.pause": "Pausieren",
    "tm.menu.unlimit": "Begrenzung aufheben",
    "tm.menu.limit": "Begrenzen",
    "tm.menu.stop": "Beenden",
    "tm.menu.forceQuit": "Beenden erzwingen",
  },
};

/// The shared task-manager translator. Import as a store: `{$t("tm.tab.processes")}`.
export const t = createTranslator(messages);

/// The file picker's message catalog (MF2, en + de). Keys are prefixed `p.` per
/// the per-app convention.
///
/// The picker is the one dialog every app on the machine borrows, so a reader who
/// set the system to German meets it more often than any single app's own screens.
/// It is hosted on the kit and rode the kit's components from the start; it simply
/// never adopted the language, which is why every string here was English until
/// this catalog existed.
///
/// The trust line is one whole sentence per case rather than a name glued to a
/// phrase: German puts the object where English does not, and the sentence that
/// tells a reader what they are about to hand over is the last one to garble.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";
export { locale, dir } from "@arlen/ui-kit/i18n";

const P = (input: string, one: string, other: string) =>
  `.input {$${input} :number}\n.match $${input}\none {{${one}}}\n*   {{${other}}}`;

const messages: Catalogs = {
  en: {
    "p.title.open": "Open file",
    "p.title.save": "Save file",
    "p.title.folder": "Choose folder",
    "p.confirm.open": "Open",
    "p.confirm.save": "Save",
    "p.confirm.folder": "Choose folder",
    "p.cancel": "Cancel",

    "p.up": "Up one directory",
    "p.hidden": "Toggle hidden files (Ctrl+H)",
    "p.hidden.show": "Show hidden files",
    "p.hidden.hide": "Hide hidden files",
    "p.filter.placeholder": "Filter",
    "p.filter.aria": "Filter the listing by name",
    "p.filter.all": "All files",
    "p.view.list": "List view",
    "p.view.grid": "Grid view",

    "p.app.unknown": "The requesting app",
    "p.trust.open": P(
      "count",
      "{$app} gets access to the file you choose",
      "{$app} gets access to the files you choose",
    ),
    "p.trust.save": "{$app} gets to save to the location you choose",
    "p.trust.folder": "{$app} gets access to the folder you choose",

    "p.save.as": "Save as",
    "p.save.placeholder": "filename",
    "p.save.in": "in {$dir}",
    "p.err.required": "Filename is required.",
    "p.err.reserved": "Reserved name.",
    "p.err.slash": "Slashes are not allowed in the filename.",
    "p.err.nul": "Filename cannot contain a NUL byte.",
    "p.err.control": "Filename cannot contain control characters.",

    "p.cap": P(
      "n",
      "Selection limited to {$n} file.",
      "Selection limited to {$n} files.",
    ),
    "p.replace": "Replace {$name}?",

    // The sidebar. The label is what a reader sees; the folder on disk keeps
    // its own name, which is why `places.ts` translates the label and never
    // the path. Same wording as the Files app, because they are the same places.
    "p.places.places": "Places",
    "p.places.recent": "Recent",
    "p.place.home": "Home",
    "p.place.documents": "Documents",
    "p.place.downloads": "Downloads",
    "p.place.pictures": "Pictures",
    "p.place.music": "Music",
    "p.place.videos": "Videos",
    "p.place.desktop": "Desktop",
  },
  de: {
    "p.title.open": "Datei öffnen",
    "p.title.save": "Datei speichern",
    "p.title.folder": "Ordner auswählen",
    "p.confirm.open": "Öffnen",
    "p.confirm.save": "Speichern",
    "p.confirm.folder": "Ordner auswählen",
    "p.cancel": "Abbrechen",

    "p.up": "Eine Ebene nach oben",
    "p.hidden": "Versteckte Dateien ein- oder ausblenden (Strg+H)",
    "p.hidden.show": "Versteckte Dateien anzeigen",
    "p.hidden.hide": "Versteckte Dateien ausblenden",
    "p.filter.placeholder": "Filtern",
    "p.filter.aria": "Liste nach Namen filtern",
    "p.filter.all": "Alle Dateien",
    "p.view.list": "Listenansicht",
    "p.view.grid": "Rasteransicht",

    "p.app.unknown": "Die anfragende App",
    "p.trust.open": P(
      "count",
      "{$app} bekommt Zugriff auf die Datei, die du auswählst",
      "{$app} bekommt Zugriff auf die Dateien, die du auswählst",
    ),
    "p.trust.save": "{$app} darf an dem Ort speichern, den du auswählst",
    "p.trust.folder": "{$app} bekommt Zugriff auf den Ordner, den du auswählst",

    "p.save.as": "Speichern als",
    "p.save.placeholder": "Dateiname",
    "p.save.in": "in {$dir}",
    "p.err.required": "Ein Dateiname fehlt.",
    "p.err.reserved": "Dieser Name ist reserviert.",
    "p.err.slash": "Schrägstriche sind im Dateinamen nicht erlaubt.",
    "p.err.nul": "Der Dateiname darf kein NUL-Byte enthalten.",
    "p.err.control": "Der Dateiname darf keine Steuerzeichen enthalten.",

    "p.cap": P(
      "n",
      "Auswahl auf {$n} Datei begrenzt.",
      "Auswahl auf {$n} Dateien begrenzt.",
    ),
    "p.replace": "{$name} ersetzen?",

    "p.places.places": "Orte",
    "p.places.recent": "Zuletzt",
    "p.place.home": "Persönlicher Ordner",
    "p.place.documents": "Dokumente",
    "p.place.downloads": "Downloads",
    "p.place.pictures": "Bilder",
    "p.place.music": "Musik",
    "p.place.videos": "Videos",
    "p.place.desktop": "Schreibtisch",
  },
};

/// The picker's translator store. Read as `$t("p.cancel")` in a component.
export const t = createTranslator(messages);

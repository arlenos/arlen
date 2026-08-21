/// The Wine manager's message catalog (MF2, en + de). Keys are prefixed `wn.` per
/// the per-app convention.
///
/// Born translatable: every sentence a person reads is a key here, in both
/// languages, from the first commit.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: {
    "wn.app.title": "Windows programs",
    "wn.none": "No bottles yet. A bottle is a Windows program with its own prefix, reaching only the folders you grant it.",
    // Said instead of the invitation above when Wine is not installed, because
    // "no bottles yet" reads as "make one" and there is nothing behind that here.
    "wn.noWine": "Wine is not installed on this machine, so no Windows program can run yet. A bottle would be a Windows program with its own prefix, reaching only the folders you grant it.",
    "wn.failed": "The bottles could not be listed: {$reason}",
    "wn.prefix": "Prefix",
    "wn.drives": "Folders this program can reach",
    "wn.noDrives": "Nothing. This program sees its own prefix and no folder of yours.",
    "wn.writable": "may write",
    "wn.readOnly": "read only",
    "wn.egress": "Network",
    // Reads under the "Network" label, so it must not repeat it: the first render
    // said "Network no network", and "Netzwerk kein Netzwerk" in German.
    "wn.egressNone": "not allowed",
    // Said out loud rather than hidden, because a bottle missing from this list
    // sends someone looking on disk for something that is sitting right there.
    "wn.unreadable": "{$path} could not be read, so that bottle is not shown above: {$reason}",
  },
  de: {
    "wn.app.title": "Windows-Programme",
    "wn.none": "Noch keine Flaschen. Eine Flasche ist ein Windows-Programm mit eigenem Prefix, das nur die Ordner erreicht, die du freigibst.",
    "wn.noWine": "Wine ist auf diesem Rechner nicht installiert, es kann also noch kein Windows-Programm laufen. Eine Flasche wäre ein Windows-Programm mit eigenem Prefix, das nur die Ordner erreicht, die du freigibst.",
    "wn.failed": "Die Flaschen waren nicht auflistbar: {$reason}",
    "wn.prefix": "Prefix",
    "wn.drives": "Ordner, die dieses Programm erreichen kann",
    "wn.noDrives": "Keine. Dieses Programm sieht sein eigenes Prefix und keinen deiner Ordner.",
    "wn.writable": "darf schreiben",
    "wn.readOnly": "nur lesen",
    "wn.egress": "Netzwerk",
    "wn.egressNone": "nicht erlaubt",
    "wn.unreadable": "{$path} war nicht lesbar, diese Flasche fehlt deshalb oben: {$reason}",
  },
};

/// Translate by key. Every sentence in this app goes through here.
export const t = createTranslator(messages);

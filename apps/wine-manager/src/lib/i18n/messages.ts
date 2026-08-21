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
    "wn.whereBottles": "Bottles are kept in {$dir}.",
    // Said instead of the invitation above when Wine is not installed, because
    // "no bottles yet" reads as "make one" and there is nothing behind that here.
    "wn.noWineWithBottles": "Wine is not installed on this machine, so none of these bottles can run a program yet. What they are granted is still shown, and still true.",
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
    // The prefix is a directory a person can open and Wine writes to on every run,
    // so what is recorded here and what the program will actually meet can part
    // ways. Said out loud rather than shown as the record alone.
    "wn.driveMissing": "{$letters} is granted here and not in the prefix, so the program cannot see that folder at all.",
    "wn.driveUnexpected": "{$letters} is in the prefix and was never granted. Something else wrote it, and it reaches wherever it points.",
    "wn.escaped": "Something in this bottle reaches out of it without a grant behind it: {$paths}",
    "wn.notBooted": "This bottle has no prefix yet, so nothing has been set up in it.",
    "wn.repair": "Put this bottle back",
    // Said on the button's own line, because "repair" invites the fear that it
    // will throw away what is inside.
    "wn.repairNote": "Closes the doors that were opened and restores the folder letters. Nothing inside the bottle is touched.",
    "wn.repaired": "This bottle matches what it says again.",
    "wn.repairFailed": "This bottle could not be put back: {$reason}",
    "wn.revoke": "Take this back",
    "wn.revokeFailed": "{$letter} could not be taken back: {$reason}",
    "wn.forget": "Forget this bottle",
    // Says where the files go, because "forget" sounds final and this is not.
    "wn.forgetNote": "Moves this bottle's files to the trash, where you can put them back.",
    "wn.forgotten": "Moved to the trash: {$path}",
    "wn.forgottenNoFiles": "This bottle had no files, so only the record was removed.",
    "wn.forgetFailed": "This bottle was kept: {$reason}",
  },
  de: {
    "wn.app.title": "Windows-Programme",
    "wn.none": "Noch keine Flaschen. Eine Flasche ist ein Windows-Programm mit eigenem Prefix, das nur die Ordner erreicht, die du freigibst.",
    "wn.whereBottles": "Flaschen liegen in {$dir}.",
    "wn.noWineWithBottles": "Wine ist auf diesem Rechner nicht installiert, keine dieser Flaschen kann also ein Programm starten. Was sie freigegeben haben, steht trotzdem hier und stimmt weiterhin.",
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
    "wn.driveMissing": "{$letters} ist hier freigegeben und nicht im Prefix, das Programm sieht diesen Ordner also gar nicht.",
    "wn.driveUnexpected": "{$letters} steht im Prefix und wurde nie freigegeben. Etwas anderes hat es geschrieben, und es reicht dorthin, wohin es zeigt.",
    "wn.escaped": "Etwas in dieser Flasche reicht ohne Freigabe aus ihr heraus: {$paths}",
    "wn.notBooted": "Diese Flasche hat noch kein Prefix, es ist also nichts darin eingerichtet.",
    "wn.repair": "Flasche zurücksetzen",
    "wn.repairNote": "Schließt die geöffneten Türen und stellt die Ordnerbuchstaben wieder her. Am Inhalt der Flasche wird nichts angerührt.",
    "wn.repaired": "Diese Flasche stimmt wieder mit dem überein, was sie sagt.",
    "wn.repairFailed": "Diese Flasche war nicht zurückzusetzen: {$reason}",
    "wn.revoke": "Zurücknehmen",
    "wn.revokeFailed": "{$letter} war nicht zurückzunehmen: {$reason}",
    "wn.forget": "Flasche vergessen",
    "wn.forgetNote": "Verschiebt die Dateien dieser Flasche in den Papierkorb, wo du sie zurückholen kannst.",
    "wn.forgotten": "In den Papierkorb verschoben: {$path}",
    "wn.forgottenNoFiles": "Diese Flasche hatte keine Dateien, es wurde nur der Eintrag entfernt.",
    "wn.forgetFailed": "Diese Flasche wurde behalten: {$reason}",
  },
};

/// Translate by key. Every sentence in this app goes through here.
export const t = createTranslator(messages);

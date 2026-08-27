/// The greeter message catalog, authored in MessageFormat 2.0. English is the source
/// of truth; German proves the reactive locale switch. Same template as the settings,
/// desktop-shell, viewers and files catalogs (I18N-R4).
///
/// **Where the greeter's language comes from is an open question.** Every other app
/// reads `~/.config/arlen/locale.toml`, which is per-user - and the greeter runs
/// before anyone has logged in, so there is no user whose choice to read. A greeter
/// wants a system-wide default (and arguably a per-profile one once a face is
/// picked). That decision is not made here; these strings are translatable either
/// way, and shipping them in one language was the thing that could not be fixed
/// later without touching every file again.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";

/// Re-exported so the app and a future language source drive the same shared store
/// instance the catalog is bound to.
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: {
    "g.app.title": "Sign in",
    "g.a11y": "Accessibility",
    "g.a11y.highContrast": "High contrast",
    "g.a11y.largerText": "Larger text",
    "g.a11y.onScreenKeyboard": "On-screen keyboard",
    "g.a11y.screenReader": "Screen reader",
    "g.a11y.readerHint": "Reading the screen aloud starts when assistive support is available.",
    "g.a11y.notRemembered": "This applies now. It could not be saved, so the next start will not have it.",
    "g.switchUser": "Switch user",
    "g.authFailed": "That did not work. Try again.",
    "g.noProfile": "No account is selected, so there is nothing to sign in to.",
    // WHY a sign-in did not happen, one per token the host returns. These were
    // English sentences written in Rust and drawn on the login screen.
    "g.why.noAccountList": "The list of accounts on this machine could not be read, so nobody can be signed in.",
    "g.why.unknownProfile": "This machine has no account by that name.",
    "g.why.noGreetd": "The login service is not reachable, so the session cannot be started.",
    "g.why.unknownSession": "That desktop session is not installed on this machine.",
    "g.why.notConnected": "This screen is not connected to the login service yet.",
    "g.factor.touchKey": "Touch your security key",
    "g.factor.useKey": "Use a security key",
    "g.factor.usePassword": "Use password instead",
    "g.osk.backspace": "Backspace",
    "g.osk.enter": "Enter",
    "g.osk.shift": "Shift",
    "g.osk.space": "Space",
    "g.password": "Password",
    "g.password.show": "Show password",
    "g.password.hide": "Hide password",
    "g.signIn": "Sign in",
    "g.power": "Power",
    "g.power.restart": "Restart",
    "g.power.shutDown": "Shut down",
    "g.power.suspend": "Suspend",
    "g.chooseProfile": "Choose a profile",
    "g.chooseSession": "Choose a session",
    "g.unreachable": "Login is not reachable right now.",
    "g.powerFailed": "That did not happen. The machine is still on.",
    "g.noProfiles": "No profiles are set up on this device.",
    "g.starting": "Starting",
  },
  de: {
    "g.app.title": "Anmelden",

    "g.a11y": "Barrierefreiheit",
    "g.a11y.highContrast": "Hoher Kontrast",
    "g.a11y.largerText": "Größere Schrift",
    "g.a11y.onScreenKeyboard": "Bildschirmtastatur",
    "g.a11y.screenReader": "Bildschirmleser",
    "g.a11y.readerHint": "Das Vorlesen beginnt, sobald die Assistenzunterstützung bereitsteht.",
    "g.a11y.notRemembered": "Gilt jetzt. Es liess sich nicht speichern, beim nächsten Start fehlt es.",
    "g.switchUser": "Benutzer wechseln",
    "g.authFailed": "Das hat nicht geklappt. Versuch es noch einmal.",
    "g.noProfile": "Es ist kein Konto ausgewählt, es gibt also nichts, wo man sich anmelden könnte.",
    "g.why.noAccountList": "Die Liste der Konten auf diesem Rechner ließ sich nicht lesen, es kann sich also niemand anmelden.",
    "g.why.unknownProfile": "Auf diesem Rechner gibt es kein Konto mit diesem Namen.",
    "g.why.noGreetd": "Der Anmeldedienst ist nicht erreichbar, die Sitzung lässt sich also nicht starten.",
    "g.why.unknownSession": "Diese Desktop-Sitzung ist auf diesem Rechner nicht installiert.",
    "g.why.notConnected": "Dieser Bildschirm ist noch nicht mit dem Anmeldedienst verbunden.",
    "g.factor.touchKey": "Berühre deinen Sicherheitsschlüssel",
    "g.factor.useKey": "Sicherheitsschlüssel verwenden",
    "g.factor.usePassword": "Stattdessen Passwort verwenden",
    "g.osk.backspace": "Rücktaste",
    "g.osk.enter": "Eingabe",
    "g.osk.shift": "Umschalt",
    "g.osk.space": "Leertaste",
    "g.password": "Passwort",
    "g.password.show": "Passwort anzeigen",
    "g.password.hide": "Passwort verbergen",
    "g.signIn": "Anmelden",
    "g.power": "Ein/Aus",
    "g.power.restart": "Neu starten",
    "g.power.shutDown": "Herunterfahren",
    "g.power.suspend": "Ruhezustand",
    "g.chooseProfile": "Profil auswählen",
    "g.chooseSession": "Sitzung auswählen",
    "g.unreachable": "Die Anmeldung ist gerade nicht erreichbar.",
    "g.powerFailed": "Das ist nicht passiert. Der Rechner l\u00e4uft weiter.",
    "g.noProfiles": "Auf diesem Gerät sind keine Profile eingerichtet.",
    "g.starting": "Startet",
  },
};

/// The bound translator: `$t("g.key", params?)`, reactive to `locale`.
export const t = createTranslator(messages);

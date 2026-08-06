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
    "g.a11y": "Accessibility",
    "g.a11y.highContrast": "High contrast",
    "g.a11y.largerText": "Larger text",
    "g.a11y.onScreenKeyboard": "On-screen keyboard",
    "g.a11y.screenReader": "Screen reader",
    "g.a11y.readerHint": "Reading the screen aloud starts when assistive support is available.",
    "g.switchUser": "Switch user",
    "g.factor.touchKey": "Touch your security key",
    "g.factor.useKey": "Use a security key",
    "g.factor.usePassword": "Use password instead",
    "g.osk.backspace": "Backspace",
    "g.osk.enter": "Enter",
    "g.osk.shift": "Shift",
    "g.osk.space": "Space",
    "g.password": "Password",
    "g.signIn": "Sign in",
    "g.power": "Power",
    "g.power.restart": "Restart",
    "g.power.shutDown": "Shut down",
    "g.power.suspend": "Suspend",
    "g.chooseProfile": "Choose a profile",
    "g.chooseSession": "Choose a session",
    "g.unreachable": "Login is not reachable right now.",
    "g.noProfiles": "No profiles are set up on this device.",
    "g.starting": "Starting",
  },
  de: {
    "g.a11y": "Barrierefreiheit",
    "g.a11y.highContrast": "Hoher Kontrast",
    "g.a11y.largerText": "Größere Schrift",
    "g.a11y.onScreenKeyboard": "Bildschirmtastatur",
    "g.a11y.screenReader": "Bildschirmleser",
    "g.a11y.readerHint": "Das Vorlesen beginnt, sobald die Assistenzunterstützung bereitsteht.",
    "g.switchUser": "Benutzer wechseln",
    "g.factor.touchKey": "Berühre deinen Sicherheitsschlüssel",
    "g.factor.useKey": "Sicherheitsschlüssel verwenden",
    "g.factor.usePassword": "Stattdessen Passwort verwenden",
    "g.osk.backspace": "Rücktaste",
    "g.osk.enter": "Eingabe",
    "g.osk.shift": "Umschalt",
    "g.osk.space": "Leertaste",
    "g.password": "Passwort",
    "g.signIn": "Anmelden",
    "g.power": "Ein/Aus",
    "g.power.restart": "Neu starten",
    "g.power.shutDown": "Herunterfahren",
    "g.power.suspend": "Ruhezustand",
    "g.chooseProfile": "Profil auswählen",
    "g.chooseSession": "Sitzung auswählen",
    "g.unreachable": "Die Anmeldung ist gerade nicht erreichbar.",
    "g.noProfiles": "Auf diesem Gerät sind keine Profile eingerichtet.",
    "g.starting": "Startet",
  },
};

/// The bound translator: `$t("g.key", params?)`, reactive to `locale`.
export const t = createTranslator(messages);

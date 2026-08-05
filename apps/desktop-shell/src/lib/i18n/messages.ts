/// The desktop-shell message catalog, authored in MessageFormat 2.0. English is the
/// source of truth; German proves the reactive locale switch. Same template as the
/// settings, system-monitor, meetings, files and harness catalogs (I18N-R4), so a
/// consumer only ever imports `t` from here.
///
/// The shell had no catalog at all until now, which is why its strings sat in the
/// i18n baseline rather than in a message file: there was nowhere for them to go.
import { createTranslator, type Catalogs } from "@arlen/ui-kit/i18n";

/// Re-exported so the shell and a future language switcher drive the same shared
/// store instance the catalog is bound to.
export { locale, dir } from "@arlen/ui-kit/i18n";

const messages: Catalogs = {
  en: {
    // Network popover
    "sh.net.title": "Network",
    "sh.net.airplaneOn": "Airplane Mode is on",
    "sh.net.airplaneHint": "Wireless connections are disabled",
    "sh.net.wifiOff": "WiFi is off",
    "sh.net.wifiOffHint": "Turn WiFi back on with the switch above",
    "sh.net.connected": "Connected",
    "sh.net.disconnected": "Disconnected",
    "sh.net.disconnect": "Disconnect",
    "sh.net.connect": "Connect",
    "sh.net.copyPassword": "Copy Password",
    "sh.net.connectionInfo": "Connection Info",
    "sh.net.ip": "IP",
    "sh.net.gateway": "Gateway",
    "sh.net.dns": "DNS",
    "sh.net.mac": "MAC",
    "sh.net.showDetails": "Show details",
    "sh.net.forget": "Forget Network",
    "sh.net.connectTo": "Connect to \"{$name}\"",
    "sh.net.password": "Password",
    "sh.net.cancel": "Cancel",
    "sh.net.scanning": "Scanning...",
    "sh.net.available": "Available Networks",
    "sh.net.refreshAria": "Refresh networks",
    "sh.net.refresh": "Refresh",
    "sh.net.noNetworks": "No networks found",
    "sh.net.vpn": "VPN",
    "sh.net.vpnActive": ".input {$count :number}\n.match $count\none {{{$count} active}}\n*   {{{$count} active}}",

    // Quick Settings panel
    "sh.qs.keyboard": "Keyboard",
    "sh.qs.navigateTiles": "Navigate tiles",
    "sh.qs.activate": "Activate",
    "sh.qs.nextFocusable": "Next focusable",
    "sh.qs.toggleHelp": "Toggle this help",
    "sh.qs.closePanel": "Close panel",

    // Bluetooth popover
    "sh.bt.title": "Bluetooth",
    "sh.bt.connecting": "Connecting...",
    "sh.bt.connected": "Connected",
    "sh.bt.paired": "Paired",
    "sh.bt.disconnect": "Disconnect",
    "sh.bt.connect": "Connect",
    "sh.bt.noAutoConnect": "Don't Auto-Connect",
    "sh.bt.autoConnect": "Auto-Connect",
    "sh.bt.forget": "Forget Device",
    "sh.bt.loading": "Loading...",
    "sh.bt.unavailable": "Bluetooth is not available on this device",
    "sh.bt.off": "Bluetooth is off",
    "sh.bt.offHint": "Turn Bluetooth back on with the switch above",
    "sh.bt.connectedSection": "Connected",
    "sh.bt.pairedDevices": "Paired Devices",
    "sh.bt.available": "Available",
  },
  de: {
    "sh.net.title": "Netzwerk",
    "sh.net.airplaneOn": "Der Flugmodus ist an",
    "sh.net.airplaneHint": "Drahtlosverbindungen sind abgeschaltet",
    "sh.net.wifiOff": "WLAN ist aus",
    "sh.net.wifiOffHint": "Schalt WLAN oben am Schalter wieder ein",
    "sh.net.connected": "Verbunden",
    "sh.net.disconnected": "Nicht verbunden",
    "sh.net.disconnect": "Trennen",
    "sh.net.connect": "Verbinden",
    "sh.net.copyPassword": "Passwort kopieren",
    "sh.net.connectionInfo": "Verbindungsdaten",
    "sh.net.ip": "IP",
    "sh.net.gateway": "Gateway",
    "sh.net.dns": "DNS",
    "sh.net.mac": "MAC",
    "sh.net.showDetails": "Details zeigen",
    "sh.net.forget": "Netzwerk vergessen",
    "sh.net.connectTo": "Mit \u201e{$name}\u201c verbinden",
    "sh.net.password": "Passwort",
    "sh.net.cancel": "Abbrechen",
    "sh.net.scanning": "Wird gesucht\u2026",
    "sh.net.available": "Verf\u00fcgbare Netzwerke",
    "sh.net.refreshAria": "Netzwerke neu suchen",
    "sh.net.refresh": "Neu suchen",
    "sh.net.noNetworks": "Keine Netzwerke gefunden",
    "sh.net.vpn": "VPN",
    "sh.net.vpnActive": ".input {$count :number}\n.match $count\none {{{$count} aktiv}}\n*   {{{$count} aktiv}}",

    "sh.qs.keyboard": "Tastatur",
    "sh.qs.navigateTiles": "Kacheln durchgehen",
    "sh.qs.activate": "Ausl\u00f6sen",
    "sh.qs.nextFocusable": "N\u00e4chstes Element",
    "sh.qs.toggleHelp": "Diese Hilfe ein-/ausblenden",
    "sh.qs.closePanel": "Panel schlie\u00dfen",

    "sh.bt.title": "Bluetooth",
    "sh.bt.connecting": "Wird verbunden\u2026",
    "sh.bt.connected": "Verbunden",
    "sh.bt.paired": "Gekoppelt",
    "sh.bt.disconnect": "Trennen",
    "sh.bt.connect": "Verbinden",
    "sh.bt.noAutoConnect": "Nicht automatisch verbinden",
    "sh.bt.autoConnect": "Automatisch verbinden",
    "sh.bt.forget": "Ger\u00e4t vergessen",
    "sh.bt.loading": "Wird geladen\u2026",
    "sh.bt.unavailable": "Bluetooth ist auf diesem Ger\u00e4t nicht verf\u00fcgbar",
    "sh.bt.off": "Bluetooth ist aus",
    "sh.bt.offHint": "Schalt Bluetooth oben am Schalter wieder ein",
    "sh.bt.connectedSection": "Verbunden",
    "sh.bt.pairedDevices": "Gekoppelte Ger\u00e4te",
    "sh.bt.available": "Verf\u00fcgbar",
  },
};

/// The bound translator: `$t("sh.key", params?)`, reactive to `locale`.
export const t = createTranslator(messages);

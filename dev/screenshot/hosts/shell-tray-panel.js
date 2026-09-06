// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Vesktop
//
// A runtime with tray clients registered, so the background-apps panel exists.
//
// Third of the four applets that do not render on a plain fixture. This one is
// the simplest of them: applet and panel ask the SAME command, unlike the sound
// pair, so one answer covers both.
//
// The list deliberately holds a `NeedsAttention` item beside two ordinary ones:
// the indicator changes on that status, and a fixture where every item is calm
// cannot show it. `tooltip_title` is null on one, which is the fall-back chain
// the shell's own comment describes - tooltip, then title, then icon name.
(function () {
  var ITEMS = [
    {
      service: "org.kde.StatusNotifierItem-4711-1",
      id: "Vesktop", category: "Communications", status: "Active",
      title: "Vesktop", icon_name: "vesktop", icon_pixmap: null,
      tooltip_title: "Vesktop", tooltip_description: "3 ungelesene Nachrichten",
      menu_path: "/MenuBar",
    },
    {
      service: "org.kde.StatusNotifierItem-5120-1",
      id: "nm-applet", category: "SystemServices", status: "NeedsAttention",
      title: "Netzwerk", icon_name: "network-wireless", icon_pixmap: null,
      tooltip_title: null, tooltip_description: null,
      menu_path: "/MenuBar",
    },
    {
      service: "org.kde.StatusNotifierItem-6033-1",
      id: "Syncthing", category: "ApplicationStatus", status: "Active",
      title: "Syncthing", icon_name: "syncthing", icon_pixmap: null,
      tooltip_title: "Syncthing", tooltip_description: "Alles synchron",
      menu_path: null,
    },
  ];

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "get_sni_items") return Promise.resolve(ITEMS);
      if (cmd === "set_popover_input_region") return Promise.resolve(null);
      if (cmd === "frontend_log") return Promise.resolve(null);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

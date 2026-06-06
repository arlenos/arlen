# desktop-shell

The Arlen desktop shell. This is the top-level UI of the system: the bar across
the top of every screen, the launcher, the system indicators, and the
notification surface. It is a multi-window Tauri 2 app with a SvelteKit and
Svelte 5 frontend, and it draws its windows as Wayland layer-shell surfaces so
they sit above normal app windows.

## What it draws

The **top bar** (a 36px layer-shell surface anchored to all four edges) carries
the global menu on the left, the workspace indicator in the centre, and the
system applets on the right: network, audio, bluetooth, battery, the system
tray, the layout mode, the clock, and the quick-settings trigger. Each applet is
an indicator plus a popover.

The **waypointer** is a separate fullscreen layer-shell window, hidden until you
tap the Super key. It is the launcher and command surface: app search, inline
math and unit and date answers, shell commands behind `>`, man pages behind `#`,
and project search.

Quick settings, the notification panel, and every popover render here too. There
is one global bar, not a title bar per window, so first-party apps draw their own
controls and GTK apps get a generated theme.

## How it talks to the rest of the system

- **Compositor**: the `arlen-shell-overlay` Wayland protocol for context menus,
  tab bars, indicators, and the Super-key launcher trigger; `wlr-foreign-toplevel`
  and `ext-workspace` for window and workspace state.
- **Notification daemon**: a Unix socket client that receives notifications and
  sends back dismiss, action, and do-not-disturb commands.
- **Knowledge daemon and Event Bus**: read-only graph queries for projects and
  focus mode, and an Event Bus consumer for window, config, and project events.
- **System services over D-Bus**: BlueZ for bluetooth, NetworkManager for
  network, UPower for battery, the StatusNotifierWatcher for the tray.

## Building

This is part of the Arlen monorepo and shares the UI kit at `sdk/ui-kit` through
the `@arlen/ui-kit` alias. Run it against a running compositor:

```
WAYLAND_DISPLAY=wayland-N cargo tauri dev
```

Use `cargo tauri dev`, not the raw binary; it needs the Vite dev server. The
Rust backend lives in `src-tauri/`, the frontend in `src/`.

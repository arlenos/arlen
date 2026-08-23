# Screenshot-verify harness (Test Layer 1b)

The "screenshot-verify loop" the coder docs mandate: render a webview headlessly
and capture a PNG you can actually look at. Drives `WebKitWebDriver` (the same
WebKit engine the Tauri apps use, `webkit2gtk` 2.52.x) under `Xvfb`, so it runs
with no display - in CI or an agent shell.

## Requirements

- `WebKitWebDriver` (Arch: `webkitgtk-6.0`; Debian: `webkit2gtk-driver`)
- `Xvfb` / `xvfb-run`
- `python3`, `curl` (stdlib only, no venv)
- For the full-app variant: `tauri-driver` (`cargo install tauri-driver`)

## Render a webview / frontend (isolates "does the UI paint")

```sh
dev/screenshot/shoot.sh <url> <out.png> [inject.js] [width] [height]
```

`<url>` is a dev-server URL (`http://localhost:1427`), a `file://`, or a `data:`
URL. `[inject.js]` is optional JS run after load + before the shot (e.g. push
state into a store so a component renders); its return value is logged.

Example - confirm the harness itself works:

```sh
dev/screenshot/shoot.sh \
  'data:text/html,<body style="margin:0;background:%2300aa00;width:100vw;height:100vh"></body>' \
  /tmp/green.png
```

This renders the frontend WITHOUT the Rust/Tauri backend, which is exactly what
isolates a render bug ("the component never paints") from a backend-wiring bug
("the data never arrives"). Tauri `invoke`/event APIs are absent in this mode,
so guard frontend code with a `tauriAvailable` check (the apps already do).

## Render a full Tauri app (Rust backend + webview together)

```sh
dev/screenshot/shoot-app.sh <app-binary> <out.png> [type-text]
```

Launches the REAL app through `tauri-driver` under `Xvfb` and screenshots it, so
it verifies the whole thing - IPC + render - not just the frontend. `[type-text]`
is typed into the app's first text input and submitted with Enter (e.g. a
terminal command), so its output renders before the shot.

The binary must serve its frontend. A debug `cargo build` targets the dev server
(`devUrl`), so run the app's `npm run dev` first; a `cargo build --release`
embeds `frontendDist` and runs standalone. Example - the terminal showing a
command's output:

```sh
(cd apps/terminal && npm run dev &)        # debug binary loads localhost:1425
dev/screenshot/shoot-app.sh \
  apps/terminal/src-tauri/target/debug/arlen-terminal /tmp/term.png "echo hi"
```

Requires `tauri-driver` (`cargo install tauri-driver`) in addition to
`WebKitWebDriver` + `Xvfb`.

## Render an app's FAILURE path (no backend behind it)

```sh
dev/screenshot/shoot-no-backend.sh <app> [route] [out.png] [width] [height]

dev/screenshot/shoot-no-backend.sh clock
dev/screenshot/shoot-no-backend.sh settings privacy/physical
```

The other two scripts show the app working: `shoot.sh` renders a URL, and
`shoot-app.sh` launches the real binary with its backend. Neither shows what a
user sees when a daemon is down, and on 8 August that turned out to be where the
bugs lived - a task manager reporting 85% memory it never measured, an enabled
07:00 alarm nobody set, a printer list offering printers to remove, a week of
activity that never happened. All of it passed `svelte-check`.

This builds the app for **production** and serves that, which is the part that
matters: the fixtures are gated on `import.meta.env.DEV`, so a dev-server render
shows the sample data and proves nothing about a real session. It uses the
extensionless route (`vite preview` will serve `privacy/physical.html` and
SvelteKit then renders a 404 in the pane, which reads as a broken page rather
than a wrong URL), and it checks that what it captured is the app rather than a
"Connection refused" page - rebuilding while a preview is up takes the server
down, and that shot is written successfully, exits 0, and is worthless.

What it cannot check is whether a label that exists is **visible from the claim
it covers**. Four of that day's fixes were wrong on the first attempt in exactly
that way: the banner was at the top of the page and the false sentence was in the
middle of it. Somebody has to look at the picture.

## Reading the name a window actually has

```sh
dev/screenshot/window-title.sh <app-binary> [locale] [expected]

dev/screenshot/window-title.sh target/release/arlen-clock-app de Uhr
```

A screenshot cannot answer this one. Every app sets `<svelte:head><title>` to
its translated name, but that is the DOCUMENT title and it never leaves the
webview; the name the topbar and the workspace overview show is the NATIVE
window title, which comes from `tauri.conf.json`. So thirteen apps had the right
name in their catalog and an English one on every surface outside their own
window, and no picture of the app could show it, because the apps draw no
titlebar (`decorations: false`). The window manager is the only witness, so this
runs the binary on its own Xvfb with a config directory it cannot escape and
asks `xdotool`.

With `expected` it asserts. Run it twice with different locales: the same
binary answering "Uhr" under `de` and "Clock" under `en` is the proof that the
title follows the language rather than being a second constant.

## Photographing something that only exists once opened

A dropdown's items, a popover's body and a dialog's contents are not in the DOM
until they are opened, so a plain render of the page cannot show what they say -
which is exactly where an empty menu tells a person they have no projects.
`render-wide.py --open <css-selector>` clicks one element, waits `--settle`, then
shoots.

It REFUSES when the selector matches nothing rather than shooting the unopened
page. That is not politeness: a screenshot of the thing not happening is the most
expensive kind of green, because it looks like evidence.

Two things that cost a shot each:

  * **`--open` takes the FIRST match.** The file manager's render harness has six
    `.ph-trigger`s and the first is a fixture, so the first attempt photographed
    sample data while claiming to show the live chain. Give the harness a stable
    hook (`data-shot="..."`) rather than counting positions.
  * **A blank frame is a failure to launch, not a result.** A solid-black shot of
    the terminal harness turned out to be vite discovering `@tauri-apps/api/mocks`
    at runtime and force-reloading the page mid-snapshot. Load the route once to
    warm the dependency, then shoot. Anything that adds a mock to a harness route
    has this on its first run.

## What this does NOT cover

- The **desktop-shell** is a Wayland layer-shell surface coupled to the
  compositor; its window state (focused app, the topbar menu's `activeWindow`)
  comes from the compositor over Wayland, so neither a webview-only shot nor the
  tauri-driver variant can reproduce that correlation - it needs the full stack
  (compositor + shell) running, captured via Layer 1a (compositor
  render-readback) or on metal.

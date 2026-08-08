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

## What this does NOT cover

- The **desktop-shell** is a Wayland layer-shell surface coupled to the
  compositor; its window state (focused app, the topbar menu's `activeWindow`)
  comes from the compositor over Wayland, so neither a webview-only shot nor the
  tauri-driver variant can reproduce that correlation - it needs the full stack
  (compositor + shell) running, captured via Layer 1a (compositor
  render-readback) or on metal.

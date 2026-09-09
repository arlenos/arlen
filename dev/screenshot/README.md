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

## A picture in `out/` carries no date on its face

`out/` holds every shot any drive ever wrote and nothing prunes it, which is
right - a photograph of a real-machine sentence taken on 21 August was the
evidence that settled a question on 5 September, and deleting it would have cost
that. But on 5 September the directory held **488 pictures, 328 of them older
than two weeks**, and two of them cost an hour between them: a printers page and
a Windows-apps page, both photographed in August, both showing sentences that had
since been fixed. Each read exactly like a fresh finding.

**So a finding from a picture is a finding about the day it was taken.** Check
`ls -l` on the file and then the source, before it is a finding. The habit costs
ten seconds; not having it produced two reports of defects that no longer
existed, and one of those was nearly written up.

**And a finding from a booted image is a finding about the COMMIT it was built
from**, which is the half of that rule I did not have written down until it caught
me. On 5 September a dogfood consent card sat over the middle of every
app-verification frame. The source has a guard that says it stands aside on
exactly that boot; the guard looked broken, and I was writing it up. It was
committed the day after the image was built. The image was 194 commits behind and
nothing in the run said so.

`dev/vm/verify.py` prints it now, under the stamp: `drift: N commit(s) since, so
anything fixed in them is NOT in this image` - or that the commit is not in this
checkout at all, when the build came from a tree this one has since rebased away.
Read that line before reading the frame. The two rules are the same rule about two
different artefacts, and the image one is easier to forget because 5 GB of file
looks like the system rather than like a photograph of it.

## Asking a rendered page what went wrong with it

Three probes, run as the `inject.js` argument above. They ask different questions
and a layout change can pass any two, which is how a header fix on 5 September
was verified with one of them and shipped drawing its second row over the
calendar:

| probe | asks |
|---|---|
| `clipped-text.js` | did an element outgrow its own box, either axis |
| `clipped-by-parent.js` | did an ancestor that clips cut a child sideways |
| `overlapping-text.js` | are two elements painted in the same place |

Each has a `<name>-control.html` beside it holding the fault it looks for AND the
near-misses it must not report. Run the control when you doubt a clean answer -
`[]` from a working probe and `[]` from a broken one are the same string, and all
three of these have returned the second at some point.

`sweep-render.sh` (or `just sweep-render <base> <locale> <path…>`) runs all three
over a list of routes at 720/1280/1920, gating each on its own control first and
refusing to sweep if any of them goes blind. A path may carry a CSS selector after
`::` for a surface behind a click. **The dev server must already be running and it
must be `vite dev`** - `vite preview` renders the source language whatever
`?locale=` says.

Three things the probes learned the hard way, all of them by somebody opening the
PNG after the numbers said something alarming:

- a **layout box is not a painted one** - a scroll container's clipped-away child
  keeps its rect exactly where the layout put it;
- **a card in front is not an overprint** - a stacked deck of notices overlaps
  almost entirely and hides nothing;
- **`text-overflow: ellipsis` does nothing on a flex or grid box**, so a
  declaration that reads like a design decision can be a hard cut mid-glyph.

## Moving a real pointer

`SHOOT_HOVER` takes JavaScript returning `{x, y}` in viewport coordinates and moves
the driver's own pointer there, just before the last `--inject` runs:

```sh
SHOOT_INJECT=type.js:read.js \
SHOOT_HOVER='(() => { const r = document.querySelector("[data-mark]").getBoundingClientRect();
                      return { x: r.left + r.width * 0.15, y: r.top + r.height / 2 }; })()' \
  dev/screenshot/shoot-app.sh "$app" out/x.png
```

An earlier inject puts the thing on screen and marks it; the last one reads what
hovering did.

**Why JS and not a CSS selector.** Both element-shaped routes were tried and the
driver refuses them for anything layered: a pointer move with an element origin
runs the interactability check, and asking for the element rect runs it too, so an
xterm row answers `element not interactable` either way - the row layer is not
what the pointer hits. The page can measure the pixel it means; the driver only
has to go there.

**And know what a hover can prove.** A canvas or WebGL renderer draws its own
hover state, so there is no DOM for a probe to find however the pointer arrived.
That is what makes the terminal link case unreachable from here, and it is a
property of the renderer rather than of the feature.

## Reading a window in German

Six apps have had a defect that only the German render showed - a column sized to
an English word, a heading that never adopted the catalogue, a sample whose rows
stayed English inside a translated panel. So a drive that reads only English is
reading half its app.

A RELEASE binary takes its language from `locale.toml`, not from a URL: the
`?locale=` hook is compiled out of a production build. Write one beside the app
and point `XDG_CONFIG_HOME` at it:

```sh
cfg="$work/config-de"
mkdir -p "$cfg/arlen"
printf '[locale]\nui = "de"\n' > "$cfg/arlen/locale.toml"
XDG_CONFIG_HOME="$cfg" SHOOT_INJECT=... dev/screenshot/shoot-app.sh "$app" out/x-de.png
```

**Assert both halves**: the German string present AND the English one absent. The
first alone passes on a half-adopted catalogue that shows both, which is exactly
the state these cases exist to catch.

**And do not grep for the obvious translation.** Two of the first three cases
written this way failed against perfectly translated windows, because the pattern
was wrong rather than the app:

- German splits the verb. "The service is not running" becomes "Der Dienst läuft
  auf diesem Rechner **nicht**", so a grep for `läuft nicht` finds nothing. Match
  a noun the sentence must contain instead.
- The catalogue's word is often not the dictionary's. "not verified" is "nicht
  **geprüft**", not "nicht bestätigt".

Read the failing detail line before believing the app is broken: it prints the
window's whole text, and the answer is usually in it.

## Watching an app put something on the bus

The other half of `--menu`. `arlen-event-emit --watch <pattern> [seconds]`
subscribes and prints `saw <type> <detail>` for everything that arrives, so a
drive can assert on the WIRE rather than on a screen. That matters for the shell
surfaces an app publishes into - badges, presence, timeline - because the render
end of those needs a focused window and therefore a compositor, while the app's
own half of the contract is fully observable here.

Two things a drive has to give the app, and the second is the one that cost a
run. `XDG_RUNTIME_DIR`, so the plugin's emitter finds the same bus; and
**`ARLEN_SESSION_ID`**, because an event belongs to a session and
`UnixEventEmitter::new` refuses to invent an id. It reports the miss through
`tracing`, which in a webview host goes nowhere at all - so an app with no
session id publishes nothing and says nothing about it. `arlen-session` mints one
per login and `arlen-run` forwards it into every confined app, so a real window
always has one. `drive-mail-badge.sh` ran green with the mailbox on screen and no
badge on the wire until it was given one.

Subscribe BEFORE starting the app. The bus fans out to whoever is registered when
an event arrives, and an app publishes its first badge as soon as its data lands.

**Assert on the absence too, where the rule is a threshold.** `--watch` counts
what arrives, which makes it the right instrument for a surface whose defect is
publishing too MUCH. `drive-terminal-ambient.sh` runs a fast command and a slow
one in the same window and asserts exactly one effect on the wire: the slow one
tints and the fast one does not. Asserting only the slow case would pass on a
terminal that washes the whole screen every time somebody types `ls`, which is
the failure that surface is one step away from at all times.

## What this does NOT cover

- The **desktop-shell** is a Wayland layer-shell surface coupled to the
  compositor; its window state (focused app, the topbar menu's `activeWindow`)
  comes from the compositor over Wayland, so neither a webview-only shot nor the
  tauri-driver variant can reproduce that correlation - it needs the full stack
  (compositor + shell) running, captured via Layer 1a (compositor
  render-readback) or on metal.

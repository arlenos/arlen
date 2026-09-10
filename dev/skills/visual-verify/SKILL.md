---
name: visual-verify
description: Look at an Arlen surface with your own eyes - render any app headless and screenshot it, sweep every surface through the layout and axe probes, drive a refusal that no route reaches, boot the image in a VM. Invoke for ANY UI / render / layout / accessibility / "does it actually look right" task, and BEFORE ever saying something cannot be verified visually.
---

# Looking is always available, and it finds what reading cannot

Nobody boots this system or watches a render on your behalf. **The headless screenshot is the only visual
channel there is** - there is no verification step waiting on a person. Saying a thing cannot be checked
visually, or turning to a backend slice instead, is avoidance rather than a constraint.

The reason this matters is not process. Six weeks of findings say the same thing: **a diff cannot show you what
the screen says.** A refusal message that reads as one careful sentence in a diff is two facts on screen. A
`text-align` fix that clears every probe renders one letter per line. A dialog that focuses its card grows a
heavy red ring nobody wrote. Every one of those was found by looking and none by reading.

## The loop

render → **look at the picture** → check it against what the surface is supposed to say → fix → **re-render**.

Two failure modes to know before you start. A probe answering `[]` means it found nothing, which is not the same
as the page being right - probes ask about collisions and boxes, never about whether a person can read the
result. And a fixture that never reached its state answers exactly like a clean page; that is what
`probe-host.sh` exists to stop.

## Which tool answers which question

Everything is in `~/Repositories/arlen/dev/`.

| Question | Tool |
|---|---|
| What does this one page look like? | `dev/screenshot/headless.sh --url <url> --out shot.png --width 1280` |
| …and what does the DOM say? | same, plus `--probe-file <js>` (the file `return`s a value) |
| Does any text collide, overflow or lose its focus ring, in German, at three widths? | `dev/screenshot/sweep-render.sh <base-url> de <path…>` |
| …across every app? | `dev/screenshot/sweep-render-all.sh [locale] [app]` |
| Does axe pass, with real colour-contrast numbers? | `dev/screenshot/sweep-axe.sh [width] [app]` (add `--axe` to `headless.sh` for one page) |
| Does Escape close what a click opened? | `dev/screenshot/sweep-escape.sh <base-url> [selector…]` - with no selectors it runs the set for the route the URL names |
| What does this app do when every backend call fails? | `dev/screenshot/shoot-no-backend.sh <app> [route] [out.png]` |
| …across every app? | `dev/screenshot/sweep-no-backend.sh [width] [app]` |
| What does a REFUSAL look like - a state no route reaches? | a host fixture plus `dev/screenshot/probe-host.sh <host> <base> <probe.js> [width] [locale]` |
| Does the whole image boot to a shell bar? | `python3 dev/vm/verify.py --image dev/mkosi/arlen.raw --require-bar --wait 90 --out shot.png` |
| Does the compositor draw a client? | `dev/screenshot/shoot-compositor.sh <out.png> <client>` **in the compositor repo** - the harness and its recipe moved there on 7 September, and that repo has its own agent; do not commit there |

`headless.sh` is the only sanctioned way to render. It owns three things that were each got wrong once: an Xvfb
with a real screen size (`xvfb-run -a` alone gives 640×480), the host session cut off (`-u WAYLAND_DISPLAY`,
`GDK_BACKEND=x11`, or GTK4 prefers the inherited Wayland display and **draws on the developer's actual screen**),
and a window manager (or `fullscreen()` is never granted, the surface stays at 200px and the render refuses with
no file). Calling `render-wide.py` yourself skips all three; a gate refuses a commit that does.

## The probes, and what each one is blind to

`dev/screenshot/*.js`, passed with `--probe-file`, or run over a whole table by the sweeps. This heading said
"the four probes" over a list of five, and `container-focus-ring.js` was in neither the count nor the list -
which is the small way a document stops being read: the number and the list disagreed, and a reader counting
files found more than both.

Four run on every route the render sweep walks:

- `clipped-text.js` - an element outgrew its own box.
- `clipped-by-parent.js` - an ancestor that clips cut a child sideways.
- `overlapping-text.js` - two elements painted in the same place.
- `no-focus-ring.js` - a control takes keyboard focus and looks no different.

A fifth runs only on host rows, where something is focused for it to read:

- `container-focus-ring.js` - a composite widget rings itself instead of the descendant its
  `aria-activedescendant` names.

And one asks a question a still picture cannot, so it has its own sweep rather than a column:

- `escape-dismisses.js` - a dismissible thing does not answer Escape. It presses the key and reports what was
  open before and after, so `sweep-escape.sh` reads the verdict rather than the sweep's usual empty-is-clean
  rule. It exists because a Settings dialog once mounted its handler on its own backdrop, which never receives
  the key - and reading the file was not enough to see it.

`open-clicked.js` is in the same directory and is not one of these: it reads whether a `--open` click landed,
for the sweep's own control, and judges nothing about a route.

They ask genuinely different questions and a layout change can pass any two. They are also all blind to
legibility: `overflow-wrap: anywhere` clears every one of them and renders a name one letter per line.

## Host fixtures: seeing the state no route reaches

Most of what a surface says only appears when something goes wrong - a refused write, an expired grant, a
daemon that is not there. Under vite those paths are unreachable, so `dev/screenshot/hosts/*.js` installs a
`window.__TAURI_INTERNALS__` that answers the page's reads and refuses the one call under test, then drives the
gesture.

Four things that cost a round each, and three checks that hold the rest:

- **Declare `// EXPECT: <the sentence on screen>`** near the top. `probe-host.sh` refuses to report anything
  unless that text is actually present, so a fixture that never reached its state is loud instead of clean.
- **Answer the WHOLE shape of every command**, not the fields your fixture cares about. A short answer makes the
  consumer throw mid-render and the page keeps whatever it was showing, which reads exactly like a live defect.
  `check-fixture-answers-whole.py` refuses a commit that gets this wrong.
- **Reject with what the caller reads.** A page that switches on `e.kind` needs a rejected object, not a string,
  or the fixture photographs the fallback branch while looking like it worked. And check which way the caller
  treats a thrown error - some treat it as success on purpose.
- **Answer the page's OTHER reads honestly.** Two refusals in one picture is evidence about neither.

**Selectors: press by structure, never by a word the catalogue owns.** This section used to say "take the one
by its class *and* its exact text", and that advice is what broke five fixtures on 10 September: they pressed a
confirm labelled `Entfernen`, the copy improved to `Widerrufen`, and every one of them clicked the opener and
then waited for a button that could never appear - silently, for as long as nobody ran them. A class, a
container, a position in a dialog (the kit renders cancel then confirm inside
`[aria-labelledby=confirm-dialog-title]`, so the confirm is the last button in it) all survive a rewording. The
one text you may match is text your own fixture typed. And remember `--open` clicks before the probe runs.

Three checks hold what a reading cannot:

- `check-fixtures-are-swept.py` - every fixture is named by a row in `sweep-render-all.sh`. Twenty were named
  by nothing until 10 September, and a fixture nobody runs looks exactly like a fixture that passes.
- `check-fixture-owns-its-text.py` - no fixture presses a sentence the catalogue owns.
- `check-fixture-expect-still-said.py` - every `// EXPECT:` is words the catalogue still carries, or words the
  fixture supplies itself.

None of them can tell whether a fixture REACHES its state. That is `probe-host.sh` and the German sweep, and
it is why both exist: the static three were written because the driven one had been refusing every fixture in
the tree for months and nothing said so.

## Sweeps are the coverage, so what is missing from them is invisible

The tables in `sweep-render-all.sh`, `sweep-axe.sh` and `sweep-escape.sh` are the list of what gets looked
at. A surface not in them is unmeasured, not clean - a mail sweep once reported "0 violations" having swept
nothing at all.

Three shapes that a route walk cannot reach on its own, all of which had real defects behind them:

- **A second view behind a click.** `route::selector` clicks first. The App-access page's by-capability pivot
  had never been rendered and held four defects, including every section header printing a raw catalogue key.
- **A dynamic route.** `/apps/[id]` matches no literal route; put a concrete id in the table.
- **A dialog.** It needs the trigger clicked, or a host fixture if it only opens after a successful write.

And a row makes a surface reachable; it does not make anyone read the picture. A red-ringed dialog sat in the
sweep for days.

## Staleness: the run that reports on code you did not write

- **A release binary built by `tauri build` embeds its frontend; a debug binary loads `devUrl`.** Screenshotting
  a debug binary means screenshotting whatever dev server holds that port.
- **`shoot-app.sh` refuses a binary older than its source** and says which file changed. That refusal is correct;
  rebuild rather than reaching for `SHOOT_ALLOW_STALE=1`.
- **`?locale=de` only takes on a DEV server.** The hook is `applyDevLocale()` and it is guarded by
  `import.meta.env.DEV`, so a `vite preview` of a production build renders English under a URL that says `de` and
  says nothing about it. `sweep-render-all.sh` starts `npm run dev` for exactly this reason; a hand-rolled render
  that starts `vite preview` instead is an English shot wearing a German filename. Settings is the app to watch,
  since it adopts the language its own way.
- **A served frontend can be older than its source** even when the binary is fresh - a `.svelte` edit with no
  rebuild renders yesterday's markup.
- **Two runs fight over one port.** `sweep-render-all.sh` derives its base port from its pid in blocks of 40 for
  exactly this. When you start a dev server by hand, pick an unusual port and take it down afterwards with
  `fuser -k -n tcp <port>`.
- **And two runs fought over one DISPLAY, which is the half that hurts.** `xvfb-run -a` looks for a free
  display and then creates its lock, and those two steps are not atomic - so two renders starting together can
  take the same `:N`, the loser's server goes away under openbox and WebKit, and BOTH hang with no output and
  no exit. Both runners now derive the display number from the pid the way the ports are derived, walking
  upward past any `/tmp/.X<n>-lock` that exists, out of one shared `lib/own-display.sh`.

  **It was one runner for a day, and that is the part worth remembering.** The fix landed in `headless.sh`,
  which READS the page, and not in `shoot.sh`, which takes the PICTURE - and a sweep runs both on every row, so
  half of each row went on racing while the harness read as fixed. It surfaced on 11 September as a knowledge
  sweep aborting at its control the moment a second render started. **A fix to a duplicated mechanism is not
  done until you have grepped for the other copies**, which in this tree means `grep -n xvfb-run
  dev/screenshot/*.sh`.

  **A stuck display reads exactly like a broken probe**, and that is the most expensive false signal this
  harness can produce. On 10 September a full sweep abandoned six apps in a row at their positive control -
  "clipped-text.js does not answer in the [...] shape", "overlapping-text.js cannot see its own control" -
  which reads as a probe that has stopped working, and the first instinct is to go and fix the probe. `ps` was
  what settled it:

      654593 xvfb-run -a ... --url file://.../clipped-by-parent-control.html
      701600 xvfb-run -a ... --url http://localhost:1438/ --out .../walk4/w4-home.png

  Two lanes, both started at 22:36, both still there at 23:10. The controls were right, the probes were fine,
  and the display was gone. **If a control that has always passed suddenly cannot see its own fixture, look for
  another render before you look at the probe.**

**`pkill -f` will kill the shell you typed it in.** `-f` matches the whole command line, and your own wrapper's
command line contains the pattern you just typed - so `pkill -f sweep.sh` from a shell running `sweep.sh` kills
that shell, and the tool reports the death as a bare exit code 144 with no explanation. Use `fuser -k -n tcp
<port>` for a server, `pkill <name>` (no `-f`) for a process by name, or `kill <pid>` from a `ps` you just read.
This bit three times in one day, twice after the rule had been written down - and three times again on 10
September, every one of them `pkill -f <port>` to stop a vite server. That shape is the trap: a port number
looks like it could not possibly match anything else, and it matches the shell you are typing in, because the
number is in the command line. `fuser -k -n tcp <port>` is one character longer.

**And `pgrep -f` has the same blind spot pointed the other way: a wait that never ends.** `until ! pgrep -f
"sweep-render-all.sh de clock"; do sleep 20; done` looks like it waits for that sweep. It waits for ever: the
shell running the loop has that string in its own command line, so `pgrep` always finds at least itself and
the negation is never true. Two of those were left spinning on 11 September, one of them long after the sweep
it was watching had finished. Match something the waiting shell does not contain - a pid from a `ps` you just
read, or the log's last line - or poll the log rather than the process table.

## Reading the picture

Ask what the surface CLAIMS, not whether it drew. The defects worth finding are claims nothing backs: a label
pointing at nothing, a banner saying a check failed where no check runs, a Remove button on a reach the daemon
will refuse, a heading printing `{$seconds}` because it was called without one, `lang="en"` over German text.

Two habits that keep paying: read the German render (a longer language finds the fixed-width column English hid),
and read your own fix in the picture - twice in one morning a fix was half a fix until it was looked at.

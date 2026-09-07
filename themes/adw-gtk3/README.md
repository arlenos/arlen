# adw-gtk3, forked for the Arlen GTK3 widget theme

Upstream: <https://github.com/lassekongo83/adw-gtk3>
Commit: `47922ed96562c002696ad2d2d80383f2ad30dc93` (7 May 2026)
Copied: 7 September 2026
Licence: LGPL-2.1 (upstream's `LICENSE`, kept here verbatim; also in `LICENSES/`)

## Why a fork and not a snapshot

`own-toolkit-themes-plan.md` says it plainly: GTK 3.20 reworked theming and even point
releases move the widget nodes, so a compiled `gtk.css` rots at the next release. adw-gtk3
is re-cut per GNOME release and so is this. What is vendored is the Sass source and nothing
compiled.

## What was taken, and what was left

Taken: `src/sass/` minus its `gtk4/` subtree, `src/assets/` (the symbolic check, dash and
bullet GTK3 draws from), `src/index.theme.in`, and `LICENSE`.

Left behind: everything GTK4. `own-toolkit-themes-plan.md` is explicit that GTK4 and
libadwaita are colours only - there is no stable widget-shape API to reach, which is why
`catppuccin/gtk` and Gradience both archived themselves in June 2024. Vendoring the GTK4
half would be carrying a promise we do not intend to keep.

## What was changed

One upstream file: `src/sass/_settings.scss`, where the seven radius variables gained
`!default` so an entry point can configure them. The change is marked at the top of that
file. Upstream's own entry points (`gtk.scss`, `gtk-dark.scss`) are untouched and still
build adw-gtk3 as upstream ships it.

Three files are ours and new: `_all-widgets.scss` (upstream's widget module list, lifted out
so the two entry points share one copy), and `arlen.scss` / `arlen-dark.scss`, which
configure the shape and then load it.

## Shape here, colour elsewhere

The compiled sheet references GTK named colours - `@window_bg_color`, `@accent_color`,
`mix(@window_fg_color,@window_bg_color,0.9)` - rather than baking hexes, which is what makes
this split work at all. So:

* **Shape** (radius, spacing, flatness) is compiled in, from `_arlen-tokens.scss`, which
  `dev/scripts/build-gtk3-theme.sh` writes into a build tree from the resolved theme.
* **Colour** stays runtime: the `@define-color` block `sdk/theme/src/gtk.rs` already writes
  to `~/.config/gtk-3.0/gtk.css`. A palette change needs no rebuild.

Do not configure colour in the entry points. It would freeze the palette into a file only a
rebuild can change, and the person's theme would stop reaching GTK3 apps.

## Building it

    dev/scripts/build-gtk3-theme.sh <out-dir> [tokens.scss]

Needs `sass` (dart-sass) on PATH. With no tokens file it uses the house defaults stated in
the script.

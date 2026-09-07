# Third-party notices

Code copied into this tree keeps its own licence and is listed here, per
`docs/architecture/copy-policy.md`. Each entry names the upstream, the exact commit taken,
and what was changed.

## adw-gtk3

* Where: `themes/adw-gtk3/`
* Upstream: <https://github.com/lassekongo83/adw-gtk3>
* Commit: `47922ed96562c002696ad2d2d80383f2ad30dc93`, taken 7 September 2026
* Licence: LGPL-2.1 (`LICENSES/LGPL-2.1-only.txt`, and `themes/adw-gtk3/LICENSE`)
* Copyright: the adw-gtk3 authors
* Changed: `src/sass/_settings.scss` gained `!default` on its seven radius variables so the
  Arlen entry points can configure the shape. Marked in that file. The GTK4 subtree was not
  copied. See `themes/adw-gtk3/README.md`.

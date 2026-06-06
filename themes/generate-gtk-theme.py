#!/usr/bin/env python3
"""
Arlen GTK4 theme generator.

Reads ~/.config/arlen/theme.toml (or a custom path) and generates
~/.config/gtk-4.0/gtk.css using the GTK template.

Usage:
    python3 generate-gtk-theme.py
    python3 generate-gtk-theme.py --theme-file /path/to/theme.toml
    python3 generate-gtk-theme.py --output /path/to/gtk.css
    python3 generate-gtk-theme.py --watch   # regenerate on theme.toml change
"""

import argparse
import os
import sys
from pathlib import Path

try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib
    except ImportError:
        print("error: requires Python 3.11+ or 'tomli' package (pip install tomli)")
        sys.exit(1)


# Built-in Panda theme defaults.
# Used as fallback for any token not specified in theme.toml.
PANDA_DEFAULTS = {
    "bg_shell":   "#09090b",
    "bg_app":     "#ffffff",
    "bg_card":    "#f5f5f7",
    "bg_overlay": "#00000080",
    "bg_input":   "#f0f0f0",
    "fg_shell":   "#fafafa",
    "fg_app":     "#09090b",
    "accent":     "#09090b",
    "accent_fg":  "#ffffff",   # text on accent color
    "border":     "#e2e2e8",
    "radius":     "6px",       # GTK uses px, not rem
}


def default_theme_path() -> Path:
    config = os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config"))
    return Path(config) / "arlen" / "theme.toml"


def default_output_path() -> Path:
    config = os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config"))
    return Path(config) / "gtk-4.0" / "gtk.css"


def template_path() -> Path:
    return Path(__file__).parent / "gtk-template.css"


def load_tokens(theme_file: Path) -> dict:
    """Load tokens from theme.toml, falling back to Panda defaults."""
    tokens = dict(PANDA_DEFAULTS)

    if not theme_file.exists():
        return tokens

    with open(theme_file, "rb") as f:
        data = tomllib.load(f)

    color = data.get("color", {})
    bg = color.get("bg", {})
    fg = color.get("fg", {})

    if bg.get("shell"):
        tokens["bg_shell"] = bg["shell"]
    if bg.get("app"):
        tokens["bg_app"] = bg["app"]
    if bg.get("card"):
        tokens["bg_card"] = bg["card"]
    if bg.get("overlay"):
        tokens["bg_overlay"] = bg["overlay"]
    if bg.get("input"):
        tokens["bg_input"] = bg["input"]
    if fg.get("shell"):
        tokens["fg_shell"] = fg["shell"]
    if fg.get("app"):
        tokens["fg_app"] = fg["app"]
    if color.get("accent"):
        tokens["accent"] = color["accent"]
    if color.get("border"):
        tokens["border"] = color["border"]

    # Convert rem radius to px for GTK (GTK doesn't support rem)
    radius = data.get("radius", "0.5rem")
    if "rem" in radius:
        try:
            rem_value = float(radius.replace("rem", "").strip())
            tokens["radius"] = f"{int(rem_value * 16)}px"
        except ValueError:
            tokens["radius"] = "8px"
    else:
        tokens["radius"] = radius

    return tokens


def generate(theme_file: Path, output_file: Path) -> None:
    """Generate gtk.css from theme.toml."""
    tmpl = template_path()
    if not tmpl.exists():
        print(f"error: template not found at {tmpl}")
        sys.exit(1)

    tokens = load_tokens(theme_file)
    template = tmpl.read_text()

    css = template.format(**tokens)

    output_file.parent.mkdir(parents=True, exist_ok=True)
    output_file.write_text(css)
    print(f"generated {output_file}")


def watch(theme_file: Path, output_file: Path) -> None:
    """Watch theme.toml and regenerate on change."""
    try:
        from watchdog.observers import Observer
        from watchdog.events import FileSystemEventHandler
    except ImportError:
        # Fall back to inotifywait if watchdog is not available
        _watch_inotify(theme_file, output_file)
        return

    class Handler(FileSystemEventHandler):
        def on_modified(self, event):
            if Path(event.src_path).name == "theme.toml":
                print(f"theme.toml changed, regenerating...")
                generate(theme_file, output_file)

        def on_created(self, event):
            if Path(event.src_path).name == "theme.toml":
                print(f"theme.toml created, generating...")
                generate(theme_file, output_file)

    generate(theme_file, output_file)
    print(f"watching {theme_file.parent} for changes (Ctrl+C to stop)")

    observer = Observer()
    observer.schedule(Handler(), str(theme_file.parent), recursive=False)
    observer.start()
    try:
        while True:
            import time
            time.sleep(1)
    except KeyboardInterrupt:
        observer.stop()
    observer.join()


def _watch_inotify(theme_file: Path, output_file: Path) -> None:
    """Fallback watcher using inotifywait."""
    import subprocess
    generate(theme_file, output_file)
    print(f"watching {theme_file} for changes (Ctrl+C to stop)")
    while True:
        subprocess.run([
            "inotifywait", "-e", "modify,create",
            str(theme_file.parent)
        ], capture_output=True)
        generate(theme_file, output_file)


def main():
    parser = argparse.ArgumentParser(description="Arlen GTK4 theme generator")
    parser.add_argument(
        "--theme-file",
        type=Path,
        default=default_theme_path(),
        help="Path to theme.toml (default: ~/.config/arlen/theme.toml)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=default_output_path(),
        help="Output path for gtk.css (default: ~/.config/gtk-4.0/gtk.css)",
    )
    parser.add_argument(
        "--watch",
        action="store_true",
        help="Watch theme.toml and regenerate on change",
    )
    args = parser.parse_args()

    if args.watch:
        watch(args.theme_file, args.output)
    else:
        generate(args.theme_file, args.output)


if __name__ == "__main__":
    main()

#!/usr/bin/env bash
# Watch theme.toml and regenerate gtk.css on change.
# Run as a systemd user service or manually in background.

THEME_TOML="$HOME/.config/arlen/theme.toml"
GTK_CSS="$HOME/.config/gtk-4.0/gtk.css"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

mkdir -p "$HOME/.config/gtk-4.0"

echo "Watching $THEME_TOML for changes..."

# Initial generation
python3 "$SCRIPT_DIR/generate-gtk-theme.py" --output "$GTK_CSS"

# Watch for changes
inotifywait -m -e modify,create "$HOME/.config/arlen/" 2>/dev/null | \
while read -r dir event file; do
    if [ "$file" = "theme.toml" ]; then
        echo "theme.toml changed, regenerating gtk.css..."
        python3 "$SCRIPT_DIR/generate-gtk-theme.py" --output "$GTK_CSS"
    fi
done

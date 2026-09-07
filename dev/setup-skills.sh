#!/usr/bin/env bash
# Link this repo's skills into the directory the harness reads.
#
# The harness loads `~/.claude/skills/<name>/SKILL.md` and nowhere else, so a
# skill authored in the tree is invisible until it is linked. Idempotent: run it
# on a fresh machine, or after adding a skill.
#
# It only ever creates or replaces a symlink it would have created itself. A real
# directory there is left alone and reported - those are the personal skills that
# are nobody's repo business, and silently replacing one with a link into this
# tree would lose it.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
dest="${CLAUDE_SKILLS_DIR:-$HOME/.claude/skills}"

mkdir -p "$dest"

linked=0
kept=0
for src in "$root"/dev/skills/*/; do
  name=$(basename "$src")
  target="$dest/$name"
  if [ -e "$target" ] && [ ! -L "$target" ]; then
    echo "kept   $name (a real directory, not ours to replace)"
    kept=$((kept + 1))
    continue
  fi
  ln -sfn "$src" "$target"
  echo "linked $name -> $src"
  linked=$((linked + 1))
done

echo "$linked skill(s) linked into $dest, $kept left alone."

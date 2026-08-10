#!/usr/bin/env bash
# The verify image must be the release image PLUS probes, never minus anything.
#
# The reason this is a gate and not a convention: a verify variant that can differ
# by OMISSION is verifying a different system than the one that ships. Every
# finding it produces would then be about an artefact nobody uses, and the failure
# is silent - the image builds, boots and lies. That is the mechanism-versus-
# assembly gap one level up, and this week has already spent three findings on it.
#
# Three checks, which together give "plus, never minus" by construction:
#
#   1. Only a phase whose name says `verify` may branch on ARLEN_VERIFY_IMAGE. If
#      a release phase could ask, it could also skip, and the verify image would
#      differ by omission.
#   2. A verify phase may not install a path a release phase installs. Same file,
#      two writers, and which one wins depends on phase order - the verify image
#      would then differ in CONTENT, not only by addition.
#   3. A verify phase may not remove anything from the image. "Plus" has no
#      removals in it.
#
# NOT covered: whether the two images are actually built from the same source, and
# whether a phase's own script does something exotic at run time that this cannot
# read (a path built by variable expansion, for one). This reads install lines
# literally; it catches the shapes anyone would actually write.
#
# Shown to fail before being trusted: `dev/scripts/test-check-verify-image.mjs`
# builds trees for each of the three and one that must stay quiet.
#
# Usage: check-verify-image.sh [repo-root]
set -uo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
phases="$root/dev/mkosi/mkosi.build.d"
flag="ARLEN_VERIFY_IMAGE"
failed=0

if [ ! -d "$phases" ]; then
    echo "verify-image: no $phases; nothing to check"
    exit 0
fi

# (1) the flag is only readable where the branch belongs.
for f in "$phases"/*; do
    [ -f "$f" ] || continue
    base=$(basename "$f")
    grep -q "$flag" "$f" 2>/dev/null || continue
    case "$base" in
        *verify*) ;;
        *)
            echo "VERIFY BRANCH IN A RELEASE PHASE: $base reads $flag." >&2
            echo "  A phase that can ask whether this is a verify build can also skip," >&2
            echo "  and then the verify image differs by omission rather than by addition." >&2
            echo "  Put the branch in a verify-named phase of its own." >&2
            failed=1
            ;;
    esac
done

# The paths each side installs, read literally off the install lines.
installed_paths() { # $1 = 'verify' | 'release'
    for f in "$phases"/*; do
        [ -f "$f" ] || continue
        base=$(basename "$f")
        case "$base" in
            *verify*) [ "$1" = verify ] || continue ;;
            *)        [ "$1" = release ] || continue ;;
        esac
        grep -hoE '\$DESTDIR"?/[a-zA-Z0-9/_.-]+' "$f" 2>/dev/null | sed 's/^\$DESTDIR"\?//'
    done | sort -u
}

verify_paths=$(installed_paths verify)
release_paths=$(installed_paths release)

# (2) no verify path may collide with a release path.
if [ -n "$verify_paths" ]; then
    clash=$(comm -12 <(printf '%s\n' "$verify_paths") <(printf '%s\n' "$release_paths"))
    if [ -n "$clash" ]; then
        echo "VERIFY PHASE OVERWRITES A RELEASE PATH:" >&2
        printf '  %s\n' $clash >&2
        echo "  Two writers for one path means the verify image differs in CONTENT," >&2
        echo "  not only by addition, and which wins depends on phase order." >&2
        failed=1
    fi
fi

# (3) no removals from a verify phase.
for f in "$phases"/*verify*; do
    [ -f "$f" ] || continue
    if grep -qE '(^|[[:space:]])rm[[:space:]]' "$f" 2>/dev/null; then
        echo "VERIFY PHASE REMOVES SOMETHING: $(basename "$f") runs rm." >&2
        echo "  \"Release plus probes\" has no removals in it." >&2
        failed=1
    fi
done

if [ "$failed" -eq 0 ]; then
    count=$(printf '%s\n' "$verify_paths" | grep -c . || true)
    echo "OK: the verify image adds $count path(s) and changes none."
fi
exit "$failed"

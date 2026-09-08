#!/usr/bin/env bash
# Run the `license` workflow's own checks locally, when the tools are here.
#
# WHY THIS IS SEPARATE FROM THE GATE SWEEP. `run-ci-gates.sh` derives its list
# from `.github/workflows/ci.yml`, which is one workflow. The licence job lives
# in `license.yml` and was therefore invisible to every local run, so the only
# way to learn that a commit broke it was the planner reading a red CI. That
# happened twice in one night: once when the adw-gtk3 fork brought an
# unreferenced LGPL text across, and once when the licence-header gate's own
# regex was parsed as an SPDX expression - a check whose subject is licensing,
# going red on licensing, with nothing local able to say so.
#
# WHY IT IS NOT A `check-*` SCRIPT. `check-gates-registered.py` requires every
# one of those to be named in `ci.yml`, and the structural-checks job there does
# not install `reuse`. Registering it would buy a gate that silently passes in
# CI, which is the failure this whole directory exists to avoid. The real run is
# `license.yml`; this is the local echo of it.
#
# It is best-effort by design: neither `reuse` nor `cargo-deny` is on a laptop by
# default, so an absent one is REPORTED and the commit proceeds. A missing tool
# must not block work, but it must not be silent either - the first cut ran only
# the REUSE half and printed one confident `ok`, from which the honest reading is
# that the licence job is green, and half of it had not been looked at.
#
# Arguments, if given, are the files being committed: the dependency-licence half
# only has an opinion about the resolved dependency graph, so a commit that
# touches no manifest cannot change its answer and does not pay for it. With no
# arguments both halves run, which is what a standalone invocation wants.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

status=0
changed=("$@")

# Does this commit touch anything the dependency-licence check reads?
manifest_touched() {
    [ ${#changed[@]} -eq 0 ] && return 0
    for f in "${changed[@]}"; do
        case "$f" in
            *Cargo.toml|*Cargo.lock|deny.toml) return 0 ;;
        esac
    done
    return 1
}

if ! command -v cargo-deny >/dev/null 2>&1; then
    printf '%-42s not run here: cargo-deny is not installed\n' "cargo deny licenses"
elif ! manifest_touched; then
    printf '%-42s not run here: no manifest in this commit\n' "cargo deny licenses"
else
    deny_failed=""
    while IFS= read -r manifest; do
        dir=$(dirname "$manifest")
        grep -q '^\[workspace\]' "$manifest" || continue
        if ! out=$(cd "$dir" && cargo deny --config "$OLDPWD/deny.toml" check licenses 2>&1); then
            deny_failed="$deny_failed$dir\n$out\n"
        fi
    done < <(git ls-files '*Cargo.toml')
    if [ -n "$deny_failed" ]; then
        printf '%-42s FAIL\n' "cargo deny licenses"
        printf '%b' "$deny_failed" | head -25
        status=1
    else
        printf '%-42s ok\n' "cargo deny licenses"
    fi
fi

if ! command -v reuse >/dev/null 2>&1; then
    printf '%-42s not run here: reuse is not installed (pipx install reuse)\n' "reuse lint"
    exit "$status"
fi

if out=$(reuse lint 2>&1); then
    printf '%-42s ok\n' "reuse lint"
    exit "$status"
fi

printf '%-42s FAIL\n' "reuse lint"
# The findings come BEFORE reuse's own summary block, so print that half rather
# than the tail: a `tail` here showed a wall of zero-counts and none of the file
# names, which is the shape of a message people stop reading.
printf '%s\n' "$out" | sed '/^# SUMMARY/,$d' | grep -v '^$' | head -25
exit 1

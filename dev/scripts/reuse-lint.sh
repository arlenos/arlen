#!/usr/bin/env bash
# Run the `license` workflow's own check locally, when the tool is here.
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
# It is best-effort by design: `reuse` is a Python tool nobody's laptop has by
# default, so an absent one is reported and the commit proceeds. A missing tool
# must not block work, but it must not be silent either.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

if ! command -v reuse >/dev/null 2>&1; then
    printf '%-42s not run here: reuse is not installed (pipx install reuse)\n' "reuse lint"
    exit 0
fi

if out=$(reuse lint 2>&1); then
    printf '%-42s ok\n' "reuse lint"
    exit 0
fi

printf '%-42s FAIL\n' "reuse lint"
# The findings come BEFORE reuse's own summary block, so print that half rather
# than the tail: a `tail` here showed a wall of zero-counts and none of the file
# names, which is the shape of a message people stop reading.
printf '%s\n' "$out" | sed '/^# SUMMARY/,$d' | grep -v '^$' | head -25
exit 1

#!/usr/bin/env bash
# Run exactly the gates the CI structural-checks step lists, in its order.
# Reading them out of the workflow rather than from memory is the point: the
# night of 9 August, a gate went red for four hours because a verification pass
# ran "the checkers" from a list in my head that was missing one.
#
# The gates are independent, so they run concurrently and their output is
# re-ordered back into CI's order afterwards. Serial, the sweep took 46 seconds,
# which is enough for a pre-commit hook to feel like something to skip - and a
# hook that gets skipped is the same as no hook. This is the difference between
# a check that runs before each commit and one that runs when someone remembers.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

# `check-*` AND `test-check-*`: the self-tests sit in the same CI step and are
# what keeps the gates honest, so a runner claiming to be "the gates CI lists"
# while skipping them was reading its own source of truth too narrowly. They cost
# under a second each and they are the reason any of the gates can be trusted.
mapfile -t scripts < <(
  grep -oE 'dev/scripts/(test-)?check-[a-z-]*\.(py|mjs|sh)' .github/workflows/ci.yml \
    | awk '!seen[$0]++'
)

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

for i in "${!scripts[@]}"; do
  script="${scripts[$i]}"
  {
    case "$script" in
      *.py) python3 "$script" >"$out/$i.log" 2>&1 ;;
      *.mjs) node "$script" >"$out/$i.log" 2>&1 ;;
      *.sh) bash "$script" >"$out/$i.log" 2>&1 ;;
    esac
    echo $? >"$out/$i.rc"
  } &
done
wait

fail=0
for i in "${!scripts[@]}"; do
  rc=$(cat "$out/$i.rc" 2>/dev/null || echo 1)
  printf '%-42s ' "${scripts[$i]}"
  if [ "$rc" -eq 0 ]; then
    echo ok
  else
    echo FAIL
    fail=1
    # The failing gate's own words, indented. A bare FAIL means looking the
    # failure up by hand, which is the friction that gets a hook disabled.
    sed 's/^/    /' "$out/$i.log" 2>/dev/null | tail -20
  fi
done
exit "$fail"

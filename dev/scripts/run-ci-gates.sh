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

# Every `dev/scripts/` script the step runs, whatever it is called.
#
# This matched `(test-)?check-[a-z-]*` until 10 August, which is a naming habit
# wearing the costume of a derivation - and it had two holes, both found by
# walking into them rather than by reading it. `[a-z-]` excludes digits, so
# `check-i18n-reactivity.mjs` had been in CI and never once in a pre-commit run,
# for no reason but the 18 in its name. And a check whose name does not begin
# `check-` was invisible entirely.
#
# The lesson is the one this week keeps paying for: derive the list, do not
# maintain it. A filter that additionally requires a naming convention is a
# maintained list again, just harder to notice, because it fails by leaving
# things out silently and the runner still prints a confident tally.
mapfile -t scripts < <(
  grep -oE 'dev/scripts/[a-z0-9-]+\.(py|mjs|sh)' .github/workflows/ci.yml \
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

#!/usr/bin/env bash
# Run exactly the gates the CI structural-checks step lists, in its order.
# Reading them out of the workflow rather than from memory is the point: the
# night of 9 August, a gate went red for four hours because a verification pass
# ran "the checkers" from a list in my head that was missing one.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
fail=0
while read -r script; do
  printf '%-42s ' "$script"
  case "$script" in
    *.py) python3 "$script" >/dev/null 2>&1 ;;
    *.mjs) node "$script" >/dev/null 2>&1 ;;
  esac && echo ok || { echo FAIL; fail=1; }
done < <(grep -o 'dev/scripts/check-[a-z-]*\.\(py\|mjs\)' .github/workflows/ci.yml | awk '!seen[$0]++')
exit "$fail"

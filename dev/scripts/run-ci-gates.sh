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
mapfile -t found < <(
  grep -oE 'dev/scripts/[a-z0-9-]+\.(py|mjs|sh)' .github/workflows/ci.yml \
    | awk '!seen[$0]++'
)

# Not everything CI runs from `dev/scripts/` is a check. `ci-system-deps.sh`
# installs packages with sudo, so deriving the list caught it and tried to apt
# on a developer's Arch laptop, which asked for a fingerprint and then failed
# three times over.
#
# The fix is NOT a list of names here - the comment above spent a paragraph on
# why that shape rots. The script declares it, and this reads the declaration,
# so the knowledge lives with the thing it describes. The skips are printed for
# the same reason: a check that quietly drops something is the failure mode this
# whole runner exists to close.
scripts=()
skipped=()
for s in "${found[@]}"; do
  reason=$(sed -n 's/^# not-a-local-gate: //p' "$s" 2>/dev/null | head -1)
  if [ -n "$reason" ]; then
    skipped+=("$s: $reason")
  else
    scripts+=("$s")
  fi
done

# AND THE GATES THAT ARE NOT SCRIPTS AT ALL. The derivation above matches
# `dev/scripts/*.{py,mjs,sh}`, which is a filter on where a gate LIVES - the same
# shape of maintained list the comment above warns about, one level up. Two of
# CI's gates are cargo binaries in `sdk/i18n` (`arlen-i18n-lint`, `arlen-rtl-lint`)
# and were therefore invisible here, so a commit could land red on them and did:
# the planner reported one on 6 September and another on 7 September, both from my
# own commits, both from a hook that had just printed a confident tally.
#
# Derived the same way, from the workflow rather than from a list here: join the
# `run:` blocks' line continuations, take every `cargo run … --bin …-lint …`
# command whole, and drop the shell redirection CI wraps it in. Warm they cost
# under three seconds together; a cold `sdk/i18n` build is slower, and that is the
# same first-run cost the rest of the hook pays for its Python.
mapfile -t lints < <(
  python3 - .github/workflows/ci.yml <<'EOF'
import re, sys

text = open(sys.argv[1], encoding="utf-8").read()
# A YAML `run: |` block keeps shell line continuations, so a command can span
# lines. Join them before matching, or the arguments are lost.
joined = re.sub(r"\\\n\s*", " ", text)
for m in re.finditer(r"(cargo run\b[^\n]*?--bin\s+[A-Za-z0-9_-]+-lint\b[^\n]*)", joined):
    cmd = m.group(1)
    # CI wraps each in `out=$(… 2>&1)`; the runner captures output itself.
    cmd = cmd.split(" 2>&1")[0].rstrip(") ")
    print(" ".join(cmd.split()))
EOF
)

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

# One list to run and one to name, so the two kinds report identically.
names=("${scripts[@]}")
for cmd in "${lints[@]}"; do
  names+=("$(sed -n 's/.*--bin \([A-Za-z0-9_-]*\).*/\1/p' <<<"$cmd")")
done

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
base=${#scripts[@]}
for j in "${!lints[@]}"; do
  i=$((base + j))
  cmd="${lints[$j]}"
  {
    eval "$cmd" >"$out/$i.log" 2>&1
    echo $? >"$out/$i.rc"
  } &
done
wait

fail=0
for i in "${!names[@]}"; do
  rc=$(cat "$out/$i.rc" 2>/dev/null || echo 1)
  printf '%-42s ' "${names[$i]}"
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
for s in "${skipped[@]}"; do
  printf '%-42s not run here: %s\n' "${s%%:*}" "${s#*: }"
done
exit "$fail"

#!/usr/bin/env bash
# The per-crate command set, owned in one place.
#
# CI and the justfile used to spell these commands out separately, and they
# drifted four times in a week: `--passWithNoTests` handed to a runner that is
# not vitest, clippy without CI's flags, nextest's exit-4 on a test-free crate
# against `cargo test`'s silence, and finally a bare `cargo check` that never
# compiled a `#[cfg(test)]` module while CI's `cargo test` did. That last one
# put four crates in the tree that `just check` called green and CI called red.
#
# Each of those read as infrastructure noise rather than a wrong answer, which
# is the dangerous part: the local gate is the main feedback loop, and a gate
# that disagrees with the one gating merges is worse than no gate. So there is
# nothing to compare any more - both callers run this.
#
# Usage: dev/check-crate.sh <check|test> <crate-path>

set -euo pipefail

mode="${1:?usage: check-crate.sh <check|test> <crate-path>}"
crate="${2:?usage: check-crate.sh <check|test> <crate-path>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$root/$crate/Cargo.toml"

[ -f "$manifest" ] || { echo "check-crate: no Cargo.toml at $crate" >&2; exit 2; }

case "$mode" in
  check)
    # `--all-targets`: tests, benches and examples too. Without it this builds
    # the lib and the bins only, so every struct literal inside a test module is
    # invisible - which is exactly how a new field on a shared type can land
    # green and break the build.
    #
    # Annotated on failure for the same reason `test` below is, and this is the
    # half that was still missing: three separate jobs (`sdk/i18n`, the systemd
    # unit gates, the webview probe) have now been reported as "red in seconds
    # with no error line", and each time the reason was sitting in the raw log
    # while the run summary said nothing. A red nobody can read gets re-run
    # rather than fixed.
    set +e
    out=$(cargo check --all-targets --manifest-path "$manifest" 2>&1)
    rc=$?
    set -e
    printf '%s\n' "$out"
    if [ "$rc" -ne 0 ]; then
      if [ -n "${GITHUB_ACTIONS:-}" ]; then
        # cargo's own first error, or an honest admission that it exited without
        # one - which is itself the finding, and points at the environment (a
        # missing system library, a linker, disk) rather than at the code.
        why=$(printf '%s' "$out" | grep -E "^(error|error\[)" | head -2 | tr '\n' ' ')
        printf '::error title=%s check::%s\n' "$crate" \
          "${why:-cargo check exited $rc with no error line - environment, not code; see the log}"
      fi
      exit "$rc"
    fi
    ;;
  test)
    # installd and knowledge tests mutate process-global state (env vars, a
    # single on-disk graph) and must run serially to avoid races.
    extra=()
    case "$crate" in
      daemons/installd|daemons/knowledge) extra=(-- --test-threads=1) ;;
    esac
    # `cargo test`, not nextest: it is what gates merges, it passes a crate with
    # no tests instead of exiting 4, and it runs the doc tests itself rather
    # than needing a second pass that has to forgive "no library targets found".
    # `--no-fail-fast` runs every target, so a failing one can be followed by
    # several passing ones - and the tail of the log is then a green summary from
    # some other binary. That is how four CI reds across two crates read as "all
    # tests pass, no error line": the failure was real and simply not last.
    #
    # So capture, stream, and on failure repeat WHAT failed at the end, where a
    # reader looks. Under GitHub Actions also emit an annotation, so the reason
    # reaches the run summary rather than only the raw log.
    set +e
    out=$(cargo test --manifest-path "$manifest" --no-fail-fast "${extra[@]}" 2>&1)
    rc=$?
    set -e
    printf '%s\n' "$out"
    if [ "$rc" -ne 0 ]; then
      echo
      echo "=== $crate: the failing targets, repeated because --no-fail-fast buries them ==="
      # A failing target prints "test result: FAILED." and names its binary in the
      # "Running ..." line above it; both are worth having in one place.
      printf '%s\n' "$out" | grep -E "^(error|error\[)|test result: FAILED|^failures:|^ +[a-zA-Z_0-9:]+$|^     Running" | tail -40
      if [ -n "${GITHUB_ACTIONS:-}" ]; then
        why=$(printf '%s' "$out" | grep -E "^error|test result: FAILED" | head -2 | tr '\n' ' ')
        printf '::error title=%s tests::%s\n' "$crate" "${why:-cargo test exited $rc with no error line - see the log}"
      fi
      exit "$rc"
    fi
    ;;
  *)
    echo "check-crate: unknown mode '$mode' (want check or test)" >&2
    exit 2
    ;;
esac

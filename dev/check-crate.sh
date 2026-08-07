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
    cargo check --all-targets --manifest-path "$manifest"
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
    cargo test --manifest-path "$manifest" --no-fail-fast "${extra[@]}"
    ;;
  *)
    echo "check-crate: unknown mode '$mode' (want check or test)" >&2
    exit 2
    ;;
esac

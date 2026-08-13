#!/usr/bin/env bash
# When a SHARED crate changes, check what that change breaks elsewhere.
#
# The gap this closes, found by walking into it on 13 Aug: the pre-commit sweep
# runs structural checks and one test, so a commit that changes a public signature
# in `sdk/permissions` passes it while leaving three other crates - and the changed
# crate's own tests - unable to compile. I committed exactly that, caught it a
# minute later by running the crate's tests, and the incident matters less than the
# class: nothing in the sweep looks at whether the tree still builds.
#
# The obvious fix is wrong. Building the tree costs thirty-five minutes cold
# (`knowledge` compiles lbug's C++), and a hook that slow gets bypassed - after
# which it protects nothing at all, which is worse than the gap.
#
# So it is narrowed by EFFECT: only a change under `sdk/` or `contracts/` can break
# a crate the commit does not touch, and only the crates that path-depend on it can
# be broken. Both are enumerable from the tree itself - excluding the build cache
# under `dev/mkosi/mkosi.builddir/`, which holds a VENDORED git checkout of this
# repo whose manifests match the same grep. Checking it would compile a stale copy
# of the tree against the new source and report failures about neither.
#
# A commit touching one daemon does no work here and stays as fast as it was.
#
# `--all-targets`, not `--lib`: the break that started this was in the changed
# crate's OWN tests, which a library check compiles right past.
#
#     dev/scripts/check-shared-signature.sh [changed-path...]
#
# With no arguments it reads the staged set, which is what the hook wants.
set -uo pipefail

root=$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
cd "$root" || exit 0

changed=("$@")
if [ ${#changed[@]} -eq 0 ]; then
    mapfile -t changed < <(git diff --cached --name-only)
fi

# The shared crates this commit touches. A crate is its Cargo.toml's directory, so
# `sdk/permissions/src/identity_store.rs` is `sdk/permissions`.
declare -A shared=()
for path in ${changed[@]+"${changed[@]}"}; do
    case "$path" in
        sdk/*|contracts/*) ;;
        *) continue ;;
    esac
    dir=$(dirname "$path")
    while [ "$dir" != "." ] && [ "$dir" != "/" ]; do
        if [ -f "$dir/Cargo.toml" ]; then
            shared["$dir"]=1
            break
        fi
        dir=$(dirname "$dir")
    done
done

if [ ${#shared[@]} -eq 0 ]; then
    exit 0
fi

# Every crate whose manifest path-depends on one of them, plus the changed crates
# themselves. A manifest names the dependency by relative path, so the crate's
# directory name is what to look for - `sdk/permissions"` matches
# `path = "../../sdk/permissions"` and not `sdk/permissions-extra`.
declare -A to_check=()
for crate in "${!shared[@]}"; do
    to_check["$crate"]=1
    while IFS= read -r manifest; do
        [ -n "$manifest" ] || continue
        to_check["$(dirname "$manifest")"]=1
    done < <(grep -rl "$crate\"" --include=Cargo.toml . 2>/dev/null \
             | grep -vE '/target/|/mkosi\.builddir/|/node_modules/' | sed 's|^\./||')
done

echo "shared crate changed (${!shared[*]}); checking ${#to_check[@]} affected crate(s)"

# lbug/Kuzu needs <cstdint> on gcc >= 13, the same as every other build here.
export CXXFLAGS="${CXXFLAGS:-} -include cstdint"

failed=()
for crate in "${!to_check[@]}"; do
    [ -f "$crate/Cargo.toml" ] || continue
    if ! out=$(cargo check --quiet --all-targets --manifest-path "$crate/Cargo.toml" 2>&1); then
        failed+=("$crate")
        printf '%s\n' "$out" | grep -E '^(error|warning: unused)' | head -5
    fi
done

if [ ${#failed[@]} -ne 0 ]; then
    echo
    echo "these crates no longer compile after the shared change: ${failed[*]}"
    echo "A shared signature is a contract; changing it is changing every caller."
    exit 1
fi

echo "every crate that depends on the changed shared code still compiles"

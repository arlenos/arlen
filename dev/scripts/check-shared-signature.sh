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
# a crate the commit does not touch, and only the crates that depend on it can be
# broken - by a cargo path dependency, or by generating WIT bindings from its
# directory, which is the same dependency written somewhere cargo cannot see. Both
# are enumerable from the tree itself - excluding the build cache
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
#
#     dev/scripts/check-shared-signature.sh --classify < cargo-output
#
# reads one crate's captured cargo output and answers `environment` or `code`.
# It exists because this gate spent one run on 8 September blaming a commit for
# breaking forty-four crates when the machine had simply run out of disk: cargo
# failed, and the gate reported the only failure it knew how to describe. A check
# that cannot tell "your change broke this" from "this could not be built at all"
# is a surface saying something it never learned.
set -uo pipefail

# Did cargo fail because of the machine rather than the code?
#
# Narrow on purpose. Each pattern is a condition under which NO commit could have
# compiled, so treating it as a verdict about the change is always wrong; anything
# outside this list stays a code failure, because guessing the other way would let
# a real break through.
classify_failure() {
    if grep -qE 'No space left on device|os error 28' -; then
        echo environment
    else
        echo code
    fi
}

if [ "${1:-}" = "--classify" ]; then
    classify_failure
    exit 0
fi

root=$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
cd "$root" || exit 0

# `--list` prints the selection and stops, so a control can pin WHICH crates a
# change reaches without paying for the compile. The selection is the half that
# rots silently: a check that quietly stopped selecting anything would exit 0
# forever and read exactly like a pass.
list_only=""
if [ "${1:-}" = "--list" ]; then
    list_only=1
    shift
fi

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

    # A manifest is not the only way to depend on a shared crate, and the one it
    # misses cost a day on 8 September: a WASM guest reaches the WIT with
    # `wit_bindgen::generate!({ path: "../../sdk/module-sdk/wit" })` and names it
    # NOWHERE in its Cargo.toml. Adding three fields to `search-result` left
    # `modules/unicode` unable to compile, and this gate said nothing because the
    # crate is not a cargo dependent of anything that changed.
    #
    # Narrow on the mechanism rather than on the mention: a source file that both
    # names the changed crate's path AND invokes the macro is generating bindings
    # from it. Grepping for the path alone would select every file with the crate
    # in a comment and make the hook slow enough to be bypassed.
    while IFS= read -r source; do
        [ -n "$source" ] || continue
        grep -q 'wit_bindgen::generate!' "$source" || continue
        dir=$(dirname "$source")
        while [ "$dir" != "." ] && [ "$dir" != "/" ]; do
            if [ -f "$dir/Cargo.toml" ]; then
                to_check["$dir"]=1
                break
            fi
            dir=$(dirname "$dir")
        done
    done < <(grep -rl "$crate" --include='*.rs' . 2>/dev/null \
             | grep -vE '/target/|/mkosi\.builddir/|/node_modules/' | sed 's|^\./||')
done

if [ -n "$list_only" ]; then
    printf '%s\n' "${!to_check[@]}" | sort
    exit 0
fi

echo "shared crate changed (${!shared[*]}); checking ${#to_check[@]} affected crate(s)"

# lbug/Kuzu needs <cstdint> on gcc >= 13, the same as every other build here.
export CXXFLAGS="${CXXFLAGS:-} -include cstdint"

failed=()
blocked=()
for crate in "${!to_check[@]}"; do
    [ -f "$crate/Cargo.toml" ] || continue
    if ! out=$(cargo check --quiet --all-targets --manifest-path "$crate/Cargo.toml" 2>&1); then
        if [ "$(printf '%s\n' "$out" | classify_failure)" = environment ]; then
            blocked+=("$crate")
            printf '%s\n' "$out" | grep -E 'No space left on device|os error 28' | head -2
            # One is enough: the machine will not have fixed itself by the next
            # crate, and forty more identical failures bury the one line that says
            # what is actually wrong.
            break
        fi
        failed+=("$crate")
        printf '%s\n' "$out" | grep -E '^(error|warning: unused)' | head -5
    fi
done

if [ ${#blocked[@]} -ne 0 ]; then
    echo
    echo "the check could not run: the build failed on the machine, not on the change"
    echo "(${blocked[*]} ran out of disk). Free space and run it again."
    exit 1
fi

if [ ${#failed[@]} -ne 0 ]; then
    echo
    echo "these crates no longer compile after the shared change: ${failed[*]}"
    echo "A shared signature is a contract; changing it is changing every caller."
    exit 1
fi

echo "every crate that depends on the changed shared code still compiles"

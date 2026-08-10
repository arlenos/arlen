#!/bin/sh
# Host orchestrator for the bootable Arlen image.
#
# The pure-Rust daemons are cross-built here with cargo-zigbuild against an older
# glibc (2.36, below Debian Trixie's 2.41) so the binaries run on the image; they
# link only glibc (verified: max GLIBC_2.34, NEEDED libm/libc/ld-linux), so no
# Debian system-lib linking is needed and a fast host build with the warm cargo
# cache is ABI-safe. The binaries are staged into mkosi.extra/usr/bin (gitignored,
# generated); their units + enable symlinks are committed source under mkosi.extra.
# The Tauri shell + the compositor link Debian system libs (WebKitGTK, Smithay's
# stack) and are built separately against the Debian sysroot, not here.
#
# Usage: dev/mkosi/build-image.sh   (then `mkosi vm` or dev/vm/ to boot it)
set -eu

here=$(cd "$(dirname "$0")" && pwd)

# A build that dies part-way leaves the half-written arlen.raw behind, and the
# next verify run boots it without complaint. That happened on 10 Aug: the disk
# filled during `systemd-repart`, the copy failed, and a 4.4G image was sitting
# there afterwards looking exactly like a real one. A verify against it reports a
# system defect that does not exist, and the hours go into chasing it.
#
# So a failed build removes its output. `dev/vm/verify.py` then says the image is
# missing, which is true and is the thing worth being told.
#
# But only a failure DURING the image write, which is why the flag exists. Almost
# everything above the mkosi call is cross-compiling daemons, and a Rust compile
# error there is by far this script's most common failure. At that moment the
# arlen.raw on disk is the PREVIOUS one, complete and bootable, and deleting it
# would turn a typo into a 40-minute rebuild - a cleanup step that destroys the
# artefact it exists to protect. The flag is set immediately before mkosi runs, so
# the removal covers exactly the window where the file can be half-written.
#
# Both directions are asserted by `dev/scripts/test-build-image-trap.mjs`, which
# was shown to fail against the delete-on-any-failure version before being trusted.
writing_image=""
trap 'status=$?; [ $status -eq 0 ] || [ -z "$writing_image" ] || { rm -f "$here/arlen.raw"; echo ">> build failed; removed the partial $here/arlen.raw" >&2; }; exit $status' EXIT
repo=$(cd "$here/../.." && pwd)
extra="$here/mkosi.extra"
target="x86_64-unknown-linux-gnu.2.36"
export PATH="$HOME/.cargo/bin:$PATH"

# crate-path:bin-name:install-dest (dest matches the unit's ExecStart=) for the
# pure-Rust daemons, extended as each is verified. The knowledge daemon (lbug C++)
# is built via the Debian-native path, not here.
daemons="
daemons/event-bus:event-bus:/usr/bin/event-bus
"

for d in $daemons; do
    crate=$(echo "$d" | cut -d: -f1)
    bin=$(echo "$d" | cut -d: -f2)
    dest=$(echo "$d" | cut -d: -f3)
    echo ">> zigbuild $bin ($crate) -> $dest"
    ( cd "$repo" && cargo zigbuild --release --target "$target" --manifest-path "$crate/Cargo.toml" )
    # cargo writes to the resolving workspace's target/; locate the freshest match.
    out=$(find "$repo" -type f -path "*/x86_64-unknown-linux-gnu/release/$bin" -printf '%T@ %p\n' \
            | sort -nr | head -1 | cut -d' ' -f2-)
    [ -n "$out" ] || { echo "!! $bin not found after build" >&2; exit 1; }
    install -Dm755 "$out" "$extra/${dest#/}"
done

# `--verify` adds the probe phase and nothing else. The relationship it must keep -
# verify is release PLUS probes, never minus - is asserted by
# `dev/scripts/check-verify-image.sh` rather than by anyone remembering it, because
# an image that differs by omission verifies a system that does not ship.
verify_args=()
if [ "${1:-}" = "--verify" ]; then
    verify_args=(-E ARLEN_VERIFY_IMAGE=1)
    echo ">> verify variant: the release image plus the probes"
fi

echo ">> mkosi build --incremental --force"
# From here on the file on disk is this run's, so a failure may leave it partial.
writing_image=1
# --incremental yes caches the post-distro base image (debootstrap + the apt
# package install) as an intermediary, so a re-run skips that whole phase and
# resumes at the build scripts (themselves warm via the persistent BUILDDIR
# cargo/npm caches). A single --force rebuilds the OUTPUT but keeps the
# incremental cache (only -ff drops it), so the slow Debian-rootfs assembly is
# paid once, not on every build.
( cd "$repo" && PATH=/usr/sbin:/sbin:$PATH mkosi --directory "$here" --incremental yes --cache-directory "$here/mkosi.cache" "${verify_args[@]}" build --force )
echo ">> image built: $here/arlen.raw"

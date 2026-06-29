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
repo=$(cd "$here/../.." && pwd)
extra="$here/mkosi.extra"
target="x86_64-unknown-linux-gnu.2.36"
export PATH="$HOME/.cargo/bin:$PATH"

# crate-path:bin-name for the pure-Rust daemons (extended as each is verified).
daemons="daemons/event-bus:event-bus"

mkdir -p "$extra/usr/bin"
for d in $daemons; do
    crate=${d%%:*}
    bin=${d##*:}
    echo ">> zigbuild $bin ($crate)"
    ( cd "$repo" && cargo zigbuild --release --target "$target" --manifest-path "$crate/Cargo.toml" )
    # cargo writes to the resolving workspace's target/; locate the freshest match.
    out=$(find "$repo" -type f -path "*/x86_64-unknown-linux-gnu/release/$bin" -printf '%T@ %p\n' \
            | sort -nr | head -1 | cut -d' ' -f2-)
    [ -n "$out" ] || { echo "!! $bin not found after build" >&2; exit 1; }
    install -Dm755 "$out" "$extra/usr/bin/$bin"
done

echo ">> mkosi build --force"
( cd "$here" && PATH=/usr/sbin:/sbin:$PATH mkosi build --force )
echo ">> image built: $here/arlen.raw"

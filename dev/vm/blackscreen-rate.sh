#!/usr/bin/env bash
# Multi-boot characterisation of the intermittent compositor black-screen (the
# software-GL / llvmpipe socket-publish race). Boots the image N times, records
# whether each boot rendered a real desktop (top bar present), and for a BLACK
# boot reads the last `init_egl:` stage marker from that boot's serial - the
# stage the compositor was in when it went silent, i.e. where the software-GL
# init hung and never published the Wayland socket.
#
# Usage: dev/vm/blackscreen-rate.sh [N]      (default N=10)
#        IMAGE=/path/to.raw dev/vm/blackscreen-rate.sh 20
#
# Each boot uses verify.py --require-bar (exit 0 = rendered desktop, non-zero =
# black / console / no-bar) + --serial-out to persist the guest serial.
set -u
N="${1:-10}"
here="$(cd "$(dirname "$0")" && pwd)"
img="${IMAGE:-$here/../mkosi/arlen.raw}"
[ -f "$img" ] || { echo "image not found: $img (build it first)"; exit 2; }
outdir="$(mktemp -d /tmp/arlen-blackscreen.XXXXXX)"
black=0
rendered=0
echo "== black-screen rate: $N boots, image=$img"
echo "== per-boot artefacts in $outdir"
for i in $(seq 1 "$N"); do
    shot="$outdir/boot-$i.png"
    ser="$outdir/boot-$i.serial.log"
    log="$outdir/boot-$i.log"
    if python3 "$here/verify.py" --image "$img" --require-bar \
            --out "$shot" --serial-out "$ser" >"$log" 2>&1; then
        rendered=$((rendered + 1))
        echo "boot $i: RENDERED"
    else
        black=$((black + 1))
        # The last init_egl marker before the guest went silent pins the hang stage.
        stage="$(grep 'init_egl:' "$ser" 2>/dev/null | tail -1)"
        pub="$(grep -c 'Listening on' "$ser" 2>/dev/null || echo 0)"
        echo "boot $i: BLACK   socket-published=$pub   last-egl: ${stage:-<no init_egl marker logged>}"
    fi
done
echo "== RESULT: $black/$N black, $rendered/$N rendered"
echo "== black-boot serials: grep -l init_egl $outdir/*.serial.log"

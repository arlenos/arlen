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
# WAIT: seconds to let the session come up (raise it under load so a slow-but-not-
# hung boot still renders - only a true socket-publish hang then counts as black).
# With verify.py's poll-until-bar probe, WAIT is a DEADLINE (a rendered boot
# returns as soon as its bar appears), so a generous ceiling costs nothing for
# fast boots but tolerates a load-delayed boot whose bar renders past ~45s -
# eliminating the single-shot false-"black" that a tight wait produced.
WAIT="${WAIT:-90}"
# LOAD: N background `yes` CPU hogs to starve the VM's software-GL init, mimicking
# the host-build contention under which the black-screen was first traced. 0 = idle.
LOAD="${LOAD:-0}"
here="$(cd "$(dirname "$0")" && pwd)"
img="${IMAGE:-$here/../mkosi/arlen.raw}"
[ -f "$img" ] || { echo "image not found: $img (build it first)"; exit 2; }
outdir="$(mktemp -d /tmp/arlen-blackscreen.XXXXXX)"
black=0
rendered=0

load_pids=()
cleanup() { for p in "${load_pids[@]:-}"; do kill "$p" 2>/dev/null; done; }
trap cleanup EXIT
if [ "$LOAD" -gt 0 ]; then
    echo "== inducing CPU load: $LOAD background hogs (host $(nproc) cores)"
    for _ in $(seq 1 "$LOAD"); do yes >/dev/null & load_pids+=("$!"); done
fi

echo "== black-screen rate: $N boots, wait=${WAIT}s, load=$LOAD, image=$img"
echo "== per-boot artefacts in $outdir"
for i in $(seq 1 "$N"); do
    shot="$outdir/boot-$i.png"
    ser="$outdir/boot-$i.serial.log"
    log="$outdir/boot-$i.log"
    if python3 "$here/verify.py" --image "$img" --require-bar --wait "$WAIT" \
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

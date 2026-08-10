#!/usr/bin/env python3
"""Headless QEMU verify channel for the Arlen image.

Boots dev/mkosi/arlen.raw under QEMU with a single virtio-gpu (software GL /
llvmpipe; no `gl=on`, so the scanout stays CPU-readable), waits for the graphical
session to come up, then captures the framebuffer over QMP `screendump` and
asserts the compositor actually rendered (a non-trivial, non-black frame). This
is the pixel-level half of the verify pass the nested harness cannot do; OCR
(tesseract) is wired so later, once the shell renders text, assertions can key on
on-screen strings.

Usage: dev/vm/verify.py [--image PATH] [--wait SECONDS] [--out PATH]
Exit 0 if the frame rendered, non-zero otherwise.
"""
import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time

OVMF_CODE = "/usr/share/edk2/x64/OVMF_CODE.4m.fd"
OVMF_VARS = "/usr/share/edk2/x64/OVMF_VARS.4m.fd"


def qmp_connect(path, deadline):
    """Connect to the QMP socket and complete the capabilities handshake."""
    while time.monotonic() < deadline:
        try:
            sock = socket.socket(socket.AF_UNIX)
            sock.connect(path)
            break
        except OSError:
            time.sleep(0.2)
    else:
        raise TimeoutError("QMP socket never accepted a connection")
    f = sock.makefile("rwb", buffering=0)
    f.readline()  # the {"QMP": {...}} greeting
    f.write(b'{"execute":"qmp_capabilities"}\n')
    f.readline()  # the {"return": {}} ack
    return sock, f


def qmp(f, execute, **arguments):
    cmd = {"execute": execute}
    if arguments:
        cmd["arguments"] = arguments
    f.write((json.dumps(cmd) + "\n").encode())
    # read until a return/error (skip async events)
    while True:
        line = f.readline()
        if not line:
            raise EOFError("QMP closed")
        msg = json.loads(line)
        if "return" in msg or "error" in msg:
            return msg


def qmp_key(f, qcode):
    """Press + release a key by qcode (e.g. 'meta_l') via input-send-event - the
    input half of the interactive ('click') verify tier, driving the guest's
    virtio input -> compositor -> shell."""
    for down in (True, False):
        qmp(f, "input-send-event", events=[
            {"type": "key", "data": {"down": down,
                                     "key": {"type": "qcode", "data": qcode}}}])


def qmp_move(f, px, py, w, h):
    """Move the pointer to (px, py) WITHOUT clicking.

    NOT a compositing probe, though it was written as one. The idea was that passing
    the cursor over a region recomposites it from the current surface buffer, so a
    stale band that cleared would be old scanout and one that survived would be
    buffer content. The compositor keeps a SEPARATE `cursor_damage_tracker`
    (`kms/surface/mod.rs`), so the cursor never forces the scene beneath it to
    redraw and the test cannot tell the two apart. Kept because moving the pointer
    without clicking is a real gap - hover states, and putting the cursor somewhere
    harmless before a capture - but do not read a ghost result out of it."""
    ax = max(0, min(0x7fff, round(px * 0x7fff / w)))
    ay = max(0, min(0x7fff, round(py * 0x7fff / h)))
    qmp(f, "input-send-event", events=[
        {"type": "abs", "data": {"axis": "x", "value": ax}},
        {"type": "abs", "data": {"axis": "y", "value": ay}}])


def qmp_click(f, px, py, w, h):
    """Left-click at pixel (px, py) on a w x h frame via the absolute pointing
    device (virtio-tablet). QEMU's abs axis is 0..0x7fff mapped to the display, so
    a pixel maps to px * 0x7fff / w. Drives the real kernel evdev -> libinput ->
    compositor -> focused surface path, exactly as a user click does (needed to
    resolve a consent dialog headlessly - keyboard cannot, and Enter-to-approve
    would be a dangerous default for a security dialog)."""
    ax = max(0, min(0x7fff, round(px * 0x7fff / w)))
    ay = max(0, min(0x7fff, round(py * 0x7fff / h)))
    abs_ev = [
        {"type": "abs", "data": {"axis": "x", "value": ax}},
        {"type": "abs", "data": {"axis": "y", "value": ay}}]
    # Establish + settle the pointer over the target surface (a bare move lets the
    # compositor warp the cursor and the webview run its hover hit-test), THEN send
    # each button edge with the abs position IN THE SAME event, so the press and
    # release are both pinned to the exact pixel rather than relying on a separately
    # buffered position - a btn edge with no co-sent position can land the click at
    # a stale spot and register as hover-only (pointerdown/up must hit the same
    # element for a click to fire).
    qmp(f, "input-send-event", events=abs_ev)
    time.sleep(0.4)
    qmp(f, "input-send-event", events=abs_ev)
    time.sleep(0.4)
    qmp(f, "input-send-event", events=abs_ev + [
        {"type": "btn", "data": {"down": True, "button": "left"}}])
    time.sleep(0.15)
    qmp(f, "input-send-event", events=abs_ev + [
        {"type": "btn", "data": {"down": False, "button": "left"}}])


def capture(f, path, x_display=None):
    """Write the guest's current frame to `path`.

    Two ways in, because the two rendering paths cannot share one. On the
    software device QMP `screendump` reads the CPU-side framebuffer directly. Under
    virgl there is no such surface - QEMU answers `no surface` - so the guest's
    scanout only exists inside QEMU's own GTK window, and the way to a PNG is to
    grab that window off the X display QEMU is drawing on.

    Returns the QMP reply for the software path, or None for the X path (where
    there is no reply to check; the caller checks the file).
    """
    if x_display is None:
        return qmp(f, "screendump", filename=path, format="png")
    # -window root rather than by name: QEMU's window title varies with the
    # machine and version, and the Xvfb display holds nothing else.
    subprocess.run(
        ["import", "-window", "root", path],
        env={**os.environ, "DISPLAY": x_display},
        check=False,
        capture_output=True,
    )
    return None


def inspect(png):
    """Return (rendered, summary) for the captured frame."""
    from PIL import Image

    img = Image.open(png).convert("RGB")
    w, h = img.size
    colors = img.getcolors(maxcolors=1 << 24) or []
    distinct = len(colors)
    total = w * h
    nonblack = sum(c for c, rgb in colors if rgb != (0, 0, 0))
    frac = nonblack / total if total else 0.0
    # rendered = more than a single flat colour AND a real fraction of non-black
    rendered = distinct > 1 and frac > 0.001
    return rendered, f"{w}x{h}, {distinct} distinct colours, {frac*100:.1f}% non-black"


def ocr(png, psm=6, crop=None, scale=1):
    """Run tesseract on the png (optionally a cropped + upscaled region)."""
    target = png
    if crop or scale != 1:
        from PIL import Image
        img = Image.open(png).convert("RGB")
        if crop:
            img = img.crop(crop)
        if scale != 1:
            img = img.resize((img.width * scale, img.height * scale))
        target = png + f".ocr{psm}.png"
        img.save(target)
    try:
        return subprocess.run(["tesseract", target, "-", "--psm", str(psm)],
                              capture_output=True, text=True, timeout=30).stdout.strip()
    except Exception:
        return ""


def top_bar_state(png):
    """Is the shell's top bar on screen? Returns (verdict, bar_row, detail) with
    verdict one of `present`, `absent`, `inconclusive`.

    This decides whether a whole boot passed, so both wrong answers are expensive,
    and the version it replaces could give either. It compared the modal colour of
    row 8 against the modal colour of the middle row and called any difference a
    bar. That is a proxy for "the shell painted a panel" and not the same
    statement:

    - A plain gradient wallpaper with **no shell at all** reads present, because
      the top of a gradient is not the middle of a gradient. A regression that
      stops the shell rendering passes the gate.
    - A bar the same colour as the wallpaper under it reads absent, and a working
      build is condemned.

    Both are pinned as fixtures in `test_frame_checks.py`. What actually
    distinguishes a panel from a wallpaper is the EDGE: a bar ends in a step at
    its bottom, within one row, while a gradient changes by a value or two per
    row over the whole screen. So look for the step.

    And where there is no step because the bar is the colour of what is under it,
    say `inconclusive` rather than `absent`. The check cannot see a bar there; that
    is not the same as the shell not rendering, and reporting it as one is how a
    working build gets condemned.
    """
    from PIL import Image
    img = Image.open(png).convert("RGB")
    w, h = img.size

    def modal_row(y):
        row = [img.getpixel((x, y)) for x in range(0, w, 4)]
        return max(set(row), key=row.count)

    def dist(a, b):
        return sum(abs(x - y) for x, y in zip(a, b))

    # The bar is a panel across the top; its bottom edge falls somewhere in the
    # first tenth of the screen even at unusual scalings.
    band = max(12, min(h // 10, 120))
    rows = [modal_row(y) for y in range(0, band + 2)]
    steps = [(dist(rows[i], rows[i + 1]), i) for i in range(len(rows) - 1)]
    step, at = max(steps)
    top = rows[2]

    # What separates a panel from a wallpaper is not how big the step is - a dark
    # bar on a dark desktop steps by 15 while a bright one steps by 92 - but that
    # everything ABOVE the step is one flat colour. A gradient has no flat band:
    # every row differs slightly from the one before it, all the way down. So test
    # the band, and let the step be any real edge at all.
    band_top = rows[: at + 1]
    flat_top = all(dist(c, top) <= 6 for c in band_top) and at >= 4
    if flat_top and step >= 6:
        return ("present", top,
                f"a flat band of {at + 1} rows ending in an edge (step {step})")
    if flat_top:
        return ("inconclusive", top,
                f"the top {at + 1} rows are one flat colour {top} with no edge "
                f"below them, so a bar painted in the colour of what is under it "
                f"would look exactly like this")
    return ("absent", top,
            f"no flat band above an edge in the top {band} rows (largest "
            f"single-row step {step} at row {at + 1}); the top shades into the "
            f"rest of the frame the way a wallpaper does")


def has_top_bar(png):
    """Back-compat boolean for callers that only branch on presence. `inconclusive`
    is not a bar: a gate must not pass on an answer the check could not give."""
    verdict, bar, _ = top_bar_state(png)
    return verdict == "present", bar, verdict


def _amber_strip(img, w, h):
    """True if the card's opaque amber header strip is on screen. Card-specific
    and definitive when present, but a card caught mid-fade drops below it, so it
    is only ever a cheap YES, never a NO."""
    for y in range(int(h * 0.32), int(h * 0.38)):
        for x in range(int(w * 0.32), int(w * 0.68), 3):
            r, g, b = img.getpixel((x, y))
            if r > 170 and 90 < g < 210 and b < 90 and r > b + 80:
                return True
    return False


def _spread(img, bx0, by0, bx1, by1):
    """Value range over a box - a flat desktop is a few adjacent values, anything
    drawn over it is not."""
    vals = [v for y in range(by0, by1, 2) for x in range(bx0, bx1, 2)
            for v in img.getpixel((x, y))]
    return max(vals) - min(vals)


def consent_dialog_state(before_png, after_png):
    """Did the consent card go away between the two frames? Returns
    (verdict, detail) with verdict one of `present`, `dismissed`, `inconclusive`.

    Both wrong answers this gate can give are expensive, so it is built to say
    "I do not know" rather than guess:

    - A false `dismissed` lets a regression through - the shell stopped resolving
      requests and the gate says fine.
    - A false `present` condemns a working build.

    The predecessor was a bool from the after-frame alone, comparing the card's
    contrast against a strip of plain desktop beside it. That asks "is the middle
    of the screen busier than the left edge", which is true of ANY window, so an
    approve that LAUNCHES something reads as the card never leaving. That is the
    observed failure: a frame with 84.5% of its pixels changed - the whole screen
    turned over - was still called "still up".

    The comparison itself is sound; what was missing is a check that its baseline
    still holds. The left strip is desktop the centred card never covers, so:

    1. The opaque amber header is card-specific: seeing it is a definitive yes.
    2. If that strip has stopped being desktop - something is drawn over it - the
       baseline is gone and no contrast comparison on this frame means anything.
       Say so instead of answering. Judged against the before-frame's own strip,
       not an absolute, because a photo wallpaper is legitimately busy and
       dismissing the modal un-dims it (a brightness change, not a new window).
    3. Otherwise compare, which is what the check always did and does correctly
       while the surroundings really are still desktop.
    """
    from PIL import Image
    before = Image.open(before_png).convert("RGB")
    after = Image.open(after_png).convert("RGB")
    w, h = after.size
    if before.size != after.size:
        return "inconclusive", f"frame size changed {before.size} -> {after.size}"

    if _amber_strip(after, w, h):
        return "present", "the card's amber header is on screen"

    card_box = (int(w * 0.33), int(h * 0.34), int(w * 0.67), int(h * 0.66))
    desk_box = (int(w * 0.06), int(h * 0.34), int(w * 0.26), int(h * 0.66))
    desk_before = _spread(before, *desk_box)
    desk_after = _spread(after, *desk_box)

    # Un-dimming a wallpaper scales its range (the modal blends toward black, so
    # losing it is under 2x); a window appearing there multiplies it. The floor
    # keeps small numbers from tripping the ratio.
    if desk_after > 60 and desk_after > desk_before * 4:
        return ("inconclusive",
                f"the strip of desktop beside the card is no longer desktop "
                f"(spread {desk_before} -> {desk_after}), so nothing on this "
                f"frame can say whether the card is still there")

    card_after = _spread(after, *card_box)
    if card_after > desk_after + 40:
        return ("present",
                f"the card's area still carries contrast "
                f"(spread {card_after} vs desktop {desk_after})")
    return ("dismissed",
            f"the card's area is desktop again "
            f"(spread {card_after} vs desktop {desk_after})")


def frame_change(a, b):
    """Fraction of pixels that differ between two frames (0..1) - used to confirm
    an input event (e.g. Super -> waypointer) actually changed what is on screen."""
    from PIL import Image, ImageChops
    ia, ib = Image.open(a).convert("RGB"), Image.open(b).convert("RGB")
    if ia.size != ib.size:
        return 1.0
    diff = ImageChops.difference(ia, ib).convert("L").point(lambda p: 255 if p > 16 else 0)
    changed = sum(c for c, v in (diff.getcolors() or []) if v)
    return changed / (ia.width * ia.height)


def card_left_the_dom(serial_path):
    """Did the shell's frontend report the consent card removed?

    This separates two failures that look identical in a screenshot and have
    nothing to do with each other. When the card is still visible after being
    answered, either the answer never got through - the click missed, or the
    shell -> broker Resolve leg is broken - or the answer got through perfectly
    and the pixels simply stayed on screen. Only the second is PR-20.

    The shell logs `[frontend] consent: card for N is out of the DOM` when the
    card is actually gone, so the serial log settles it. Measured on the
    2026-08-09 image: that line is present and the card is still in the capture,
    so the resolve leg is fine and the stale pixels are the whole bug. Before
    this, the gate blamed the resolve leg and would have sent the next reader to
    the wrong half of the system.
    """
    if not serial_path or not os.path.exists(serial_path):
        return None
    try:
        with open(serial_path, "r", errors="replace") as fh:
            return "is out of the DOM" in fh.read()
    except OSError:
        return None


def ghost_or_resolve(serial_path, how):
    """The failure line for a card that is still on screen after `how`."""
    gone = card_left_the_dom(serial_path)
    if gone:
        return ("VERIFY FAIL: the consent card is still on screen after " + how +
                ", but the shell reports it out of the DOM - the request WAS "
                "resolved and the pixels are stale. This is PR-20: the last "
                "frame painted still contains the card and nothing repaints "
                "after the removal.")
    if gone is None:
        return ("VERIFY FAIL: the consent dialog did not dismiss after " + how +
                " (no serial log to tell a failed resolve from a stale frame)")
    return ("VERIFY FAIL: the consent dialog did not dismiss after " + how +
            " (the shell never reported the card leaving the DOM, so the "
            "request was not resolved - the click landing or the shell -> "
            "broker Resolve leg is off)")


def main():
    ap = argparse.ArgumentParser()
    here = os.path.dirname(os.path.abspath(__file__))
    ap.add_argument("--image", default=os.path.join(here, "..", "mkosi", "arlen.raw"))
    ap.add_argument("--wait", type=int, default=40, help="seconds to let the session come up")
    ap.add_argument("--out", default=os.path.join(here, "shot.png"))
    ap.add_argument("--serial-out", default=None, metavar="PATH",
                    help="persist the guest serial log to PATH (else it is discarded "
                         "with the temp dir). Used by the black-screen multi-boot "
                         "characterisation to read which init_egl stage marker was "
                         "last before a black boot.")
    ap.add_argument("--gpu", action="store_true",
                    help="render through the host GPU (virgl + egl-headless) instead of llvmpipe")
    ap.add_argument("--require-bar", action="store_true",
                    help="fail unless the shell's top bar is present (full-desktop gate)")
    ap.add_argument("--super", dest="press_super", action="store_true",
                    help="after verifying, press Super and capture a second shot "
                         "(the waypointer/launcher) to exercise the input->shell path")
    ap.add_argument("--hover", default=None,
                    help="with --super, sweep the pointer down this column after "
                         "typing (X or X,Y1,Y2,...) and capture. A stale band that "
                         "clears where the cursor passed was never in the buffer.")
    ap.add_argument("--type-keys", nargs="*", default=["f", "i"],
                    help="with --super, the qcodes to type into the open overlay, "
                         "one capture per keystroke except the last. More keys mean "
                         "more list-shrinks, which is how a stale strip that "
                         "ACCUMULATES is told apart from one wrong frame.")
    ap.add_argument("--dismiss-with-escape", action="store_true",
                    help="with --super, close the waypointer with Escape instead of "
                         "Super. Tells whether Escape reaches a shell webview at "
                         "all, which is the open half of the consent card that "
                         "will not deny on Escape")
    ap.add_argument("--app", default=None, metavar="BINARY",
                    help="launch a daily-driver app (its binary name, e.g. "
                         "arlen-system-monitor) in the booted session via QEMU fw_cfg, "
                         "so its window renders for the screenshot (TIER-A 1b). Use a "
                         "longer --wait so the app has time to come up after the shell")
    ap.add_argument("--webkit-compositing", action="store_true",
                    help="boot with WebKit's accelerated compositing left ON "
                         "(the session disables it by default for the VM's software "
                         "GL). Use to tell whether an overlay's leftover pixels are a "
                         "webview defect or a consequence of running uncomposited. "
                         "Apps may paint black under llvmpipe this way - that outcome "
                         "is itself the answer that this cannot be measured here")
    ap.add_argument("--click", action="append", default=None, metavar="X,Y",
                    help="after the wait, click these 1280x800 coordinates in order "
                         "(repeatable) and capture an after-shot. Written to answer a "
                         "question a boot alone cannot: whether a pane that renders "
                         "empty is empty because the read found nothing or because it "
                         "ran before the data existed - clicking away and back "
                         "re-mounts the view without rebuilding the image")
    ap.add_argument("--click-settle", type=int, default=8, metavar="SECONDS",
                    help="with --click, how long to wait before the after-shot. The "
                         "default is generous on purpose: the shot is evidence about "
                         "what the surface ANSWERED, and a pane still loading looks "
                         "exactly like a pane that answered nothing")
    ap.add_argument("--require-app-text", default=None, metavar="SUBSTR",
                    help="with --app, fail unless the screenshot OCRs a substring "
                         "(case-insensitive), e.g. a process name the app must show")
    ap.add_argument("--require-ai", action="store_true",
                    help="fail unless the AI layer came up: the journal (forwarded to "
                         "serial) must show the llama engine + the AI session daemons started")
    ap.add_argument("--require-dogfood", action="store_true",
                    help="fail unless the in-VM KG-AI dogfood completed: the serial "
                         "journal must show 'DOGFOOD OK' (event injected + AI completion). "
                         "Implies the AI layer; use a longer --wait (the loop waits a "
                         "promotion pass then asks, with retry for model-load latency)")
    ap.add_argument("--require-consent", action="store_true",
                    help="fail unless the release consent path is live: the serial "
                         "journal must show 'DOGFOOD CONSENT ok' (an attested app "
                         "raised a run_command-shaped request AND the broker accepted "
                         "it in a RELEASE image, past the debug-only dev.* admission). "
                         "Also reports, best-effort, whether the shell rendered the "
                         "dialog (OCR of the frame). Implies the dogfood is present")
    ap.add_argument("--approve-consent", action="store_true",
                    help="with the consent dialog up, click 'Allow once' via the "
                         "absolute pointer and confirm the dialog dismisses (the "
                         "shell -> broker Resolve leg). Implies --require-consent")
    ap.add_argument("--deny-consent", action="store_true",
                    help="with the consent dialog up, press Escape (the always-"
                         "available deny) and confirm the dialog dismisses - "
                         "exercises the keyboard path + the shell -> broker Resolve "
                         "leg via Deny. Implies --require-consent")
    args = ap.parse_args()
    if args.approve_consent or args.deny_consent:
        args.require_consent = True

    image = os.path.abspath(args.image)
    # Say how old the image is. A VM check reports on whatever was last built into
    # the image, which is not necessarily the tree in front of you - the compositor
    # harness screenshotted a six-week-old binary on 6 August and called it a pass.
    if os.path.exists(image):
        built = time.strftime("%Y-%m-%d %H:%M", time.localtime(os.path.getmtime(image)))
        age_h = (time.time() - os.path.getmtime(image)) / 3600
        print(f"image: {image} (built {built}, {age_h:.1f}h ago)", flush=True)
    if not os.path.exists(image):
        sys.exit(f"image not found: {image} (run dev/mkosi/build-image.sh first)")

    tmp = tempfile.mkdtemp(prefix="arlen-verify-")
    vars_fd = os.path.join(tmp, "OVMF_VARS.fd")
    shutil.copyfile(OVMF_VARS, vars_fd)
    qmp_path = os.path.join(tmp, "qmp.sock")
    serial = os.path.join(tmp, "serial.log")
    out = os.path.abspath(args.out)

    # Boot a throwaway qcow2 overlay backed by the raw, never the raw directly: the
    # guest filesystem is writable, so a prior boot otherwise persists its runtime
    # state into the image (the KG SQLite + graph store, the audit ledger, and the
    # dogfood's /var/lib/arlen-work/.git project signal). That persistence breaks the
    # dogfood's fresh-system assumption: on a second boot the project already exists
    # when the file is promoted, so promotion links the file and the executor's
    # tag-untagged-files workflow finds nothing untagged to write. The overlay gives
    # every run a pristine view and discards its writes, so the raw stays clean and
    # each verify is independent.
    overlay = os.path.join(tmp, "overlay.qcow2")
    subprocess.run(
        ["qemu-img", "create", "-f", "qcow2", "-b", image, "-F", "raw", overlay],
        check=True,
        stdout=subprocess.DEVNULL,
    )

    # The virgl path has no CPU-side framebuffer for QMP to dump, so QEMU must
    # draw into something we can grab: an Xvfb of our own, with QEMU's GTK display
    # and GL on. Software rendering keeps the old headless path untouched.
    x_display = None
    xvfb = None
    if args.gpu:
        x_display = ":%d" % (90 + os.getpid() % 8)
        xvfb = subprocess.Popen(
            ["Xvfb", x_display, "-screen", "0", "1280x800x24"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        time.sleep(2)

    qemu = [
        # 4 GiB + 4 vCPUs: the baked llama-server loads a ~0.8 GB GGUF and runs CPU
        # inference alongside the compositor + shell + the AI daemons, which 2 GiB /
        # 2 vCPUs cannot hold (the desktop-only verify used less). More cores cut the
        # 1B model's first-token latency so the dogfood does not time out.
        "qemu-system-x86_64", "-machine", "q35,accel=kvm:tcg", "-m", "4096", "-smp", "4",
        # Pass the host CPU through (not the feature-masked qemu64 default): the
        # baked llama-server is built with GGML's AVX2/FMA SIMD, and the default
        # virtual CPU masks those, so llama dies with SIGILL (status 4/ILL) before
        # it can serve. -cpu host gives the guest the real instruction set, which
        # is also what a real install runs on. (KVM is required for `host`; the
        # accel line already prefers it.)
        "-cpu", "host",
        "-drive", f"if=pflash,unit=0,format=raw,readonly=on,file={OVMF_CODE}",
        "-drive", f"if=pflash,unit=1,format=raw,file={vars_fd}",
        "-drive", f"if=virtio,format=qcow2,file={overlay}",
        # single virtio-gpu, no default VGA: cosmic-comp gets one DRM device with a
        # render node + GBM, and screendump captures that scanout. No gl=on, so the
        # framebuffer is CPU-readable (llvmpipe does the GL).
        # --gpu swaps llvmpipe for the host GPU through virgl. It exists for one
        # question: the shell overlays leave their last frame on screen when they
        # close, and whether that is a software-rasteriser artifact or something
        # every user would see decides how serious it is. A ghost that survives
        # this switch is not a VM artifact.
        "-vga", "none",
        "-device", "virtio-gpu-gl-pci" if args.gpu else "virtio-gpu-pci",
        # An absolute pointing device so QMP input-send-event abs clicks land
        # (the default q35 mouse is PS/2 relative, which has no fixed origin to
        # click a known pixel). Harmless when no click is driven.
        "-device", "virtio-tablet-pci",
        # egl-headless renders through the host's render node and still lets
        # screendump read the scanout, which -display gtk,gl=on would not do
        # headlessly. Without gl the virgl device has nothing to render on.
        "-display", "gtk,gl=on,window-close=off" if args.gpu else "none",
        "-qmp", f"unix:{qmp_path},server,nowait",
        "-serial", f"file:{serial}",
        "-no-reboot",
    ]
    # TIER-A 1b: request the session launch a daily-driver app for the screenshot.
    # The app's binary name rides the SMBIOS system SKU; arlen-session reads it from
    # /sys/class/dmi/id/product_sku (built-in DMI driver, no kernel module) and
    # launches the (sanitised, installed) binary after the shell. Empty SKU on a
    # normal boot, so nothing extra launches.
    # SKU and family ride in the same type=1 record: QEMU takes one -smbios entry
    # per type, so passing them separately would silently drop the first.
    smbios_fields = []
    if args.app:
        smbios_fields.append(f"sku={args.app}")
    if args.webkit_compositing:
        smbios_fields.append("family=webkit-compositing")
    if smbios_fields:
        qemu += ["-smbios", "type=1," + ",".join(smbios_fields)]
    print("+ " + " ".join(qemu))
    qemu_env = {**os.environ, "DISPLAY": x_display} if x_display else None
    proc = subprocess.Popen(qemu, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, env=qemu_env)
    try:
        sock, f = qmp_connect(qmp_path, time.monotonic() + 30)
        print(f"QMP connected; letting the session come up ({args.wait}s)...")
        # For the pure top-bar gate (the black-screen rate measurement), POLL for the
        # bar rather than a single wait-then-shot. Under heavy load the shell's WebKit
        # render can finish just after a fixed wait, which one shot mis-reports as
        # black; polling reports "did the bar render WITHIN N seconds" and the elapsed
        # time is the true time-to-render. The app/consent/super modes keep the fixed
        # wait their after-steps depend on.
        # Set when the poll actually saw the bar, so a modal covering the final
        # frame cannot erase that evidence.
        bar_seen_while_polling = False
        bar_gate_only = args.require_bar and not (
            args.app or args.press_super or args.deny_consent or args.approve_consent
            or args.click
        )
        if bar_gate_only:
            deadline = time.monotonic() + args.wait
            appeared_at = None
            while time.monotonic() < deadline:
                shot = capture(f, out, x_display)
                if "error" not in shot:
                    for _ in range(50):
                        if os.path.exists(out) and os.path.getsize(out) > 0:
                            break
                        time.sleep(0.1)
                    try:
                        if (os.path.exists(out) and os.path.getsize(out) > 0
                                and has_top_bar(out)[0]):
                            appeared_at = args.wait - (deadline - time.monotonic())
                            break
                    except Exception:
                        pass  # a truncated/mid-write PNG: poll again
                time.sleep(3)
            if appeared_at is not None:
                bar_seen_while_polling = True
                print(f"top bar appeared after {appeared_at:.0f}s (polled)")
            else:
                print(f"top bar did not appear within {args.wait}s (polled)")
        else:
            # The fixed-wait modes still watch for the bar, they just do not cut the
            # wait short: the after-steps depend on its full length. Without this a
            # consent run had no bar evidence at all, because the only sighting it
            # could have made happens seconds before the card covers the screen.
            watch_deadline = time.monotonic() + args.wait
            while time.monotonic() < watch_deadline:
                time.sleep(3)
                if bar_seen_while_polling:
                    continue
                try:
                    shot = capture(f, out, x_display)
                    if "error" not in shot and os.path.exists(out) \
                            and os.path.getsize(out) > 0 and has_top_bar(out)[0]:
                        bar_seen_while_polling = True
                        print("top bar: seen before the after-step covered it")
                except Exception:
                    pass  # a truncated/mid-write PNG: look again next round
        if args.app:
            # The shell may show a modal (e.g. its consent fixture) over the desktop;
            # press Escape so a launched app window is not hidden behind it.
            qmp_key(f, "esc")
            time.sleep(1.5)
        res = capture(f, out, x_display)
        # Only the QMP path has a reply to inspect; the X grab reports through the
        # file, which the wait below already checks.
        if res is not None and "error" in res:
            sys.exit(f"screendump failed: {res['error']}")
        # screendump is async-completed on older QEMU; give it a moment + settle.
        for _ in range(50):
            if os.path.exists(out) and os.path.getsize(out) > 0:
                break
            time.sleep(0.1)
        if args.press_super:
            # Shoot the same screen twice before touching anything. The dismissal
            # verdict below is a diff against this pre-open frame, so it is only
            # meaningful if the screen had stopped changing on its own - a boot
            # that is still settling (a consent dialog about to appear, the shell
            # finishing its first paint) moves more pixels than any ghost, and the
            # number then reports the boot rather than the overlay. Measured: a 70s
            # wait gave "84.3% still differs" on a frame that was visibly clean.
            settle = out + ".settle.png"
            time.sleep(2)
            capture(f, settle, x_display)
            for _ in range(50):
                if os.path.exists(settle) and os.path.getsize(settle) > 0:
                    break
                time.sleep(0.1)
            settled = frame_change(out, settle) if os.path.exists(settle) else 1.0

            after = out + ".after.png"
            qmp_key(f, "meta_l")            # Super: the compositor's waypointer toggle
            time.sleep(2)
            capture(f, after, x_display)
            for _ in range(50):
                if os.path.exists(after) and os.path.getsize(after) > 0:
                    break
                time.sleep(0.1)
            # Type into the open overlay and shoot again. The dismissal check below
            # asks whether a CLOSED overlay leaves its frame behind; this asks the
            # other half - whether an overlay that REDRAWS leaves the previous frame
            # under the new one. The doubled "Applications" group someone found in a
            # waypointer shot is that shape: the shell renders one such heading, so
            # two on screen means an intermediate frame survived a repaint.
            # One capture per keystroke, because the interesting question is not
            # whether a stale strip appears but whether strips ACCUMULATE.
            #
            # Each character filters the list further, so the card shrinks again and
            # vacates more. If every shrink leaves another band and the bands hold
            # successive older frames, the client's buffer is fine and its DAMAGE
            # under-reports the vacated region. A wrong buffer, or a DOM that keeps
            # old rows, would not stack a history - it would show one wrong thing.
            # That distinction is the whole question, and it costs two more
            # keystrokes rather than a compositor experiment and an image rebuild.
            typed = out + ".typed.png"
            for i, qcode in enumerate(args.type_keys):
                qmp_key(f, qcode)
                time.sleep(0.4)
                if i + 1 < len(args.type_keys):
                    time.sleep(1.5)
                    capture(f, f"{out}.typed-{i + 1}.png", x_display)
            time.sleep(2)
            capture(f, typed, x_display)
            if args.hover:
                parts = [int(v) for v in args.hover.split(",")]
                col = parts[0]
                rows = parts[1:] or [560, 600, 640]
                for py in rows:
                    qmp_move(f, col, py, 1280, 800)
                    time.sleep(0.5)
                time.sleep(1.5)
                capture(f, out + ".hover.png", x_display)
            for _ in range(50):
                if os.path.exists(typed) and os.path.getsize(typed) > 0:
                    break
                time.sleep(0.1)
            for _ in range(50):
                if os.path.exists(after) and os.path.getsize(after) > 0:
                    break
                time.sleep(0.1)
            # Close it again and capture a third frame. The overlay ghost (PR-20)
            # is a CLOSE-time artifact - the last delivered frame stays on screen -
            # so opening one proves nothing about it. Toggling back and comparing
            # against the pre-open desktop is what makes the ghost visible: if the
            # overlay's pixels are still there, the compositor kept them.
            dismissed = out + ".dismissed.png"
            # Super by default, because that is the toggle. `--dismiss-with-escape`
            # sends Escape instead, and it is not a variant for its own sake: the
            # consent card measured on 2026-08-09 never resolves on Escape while
            # holding an exclusive keyboard grab with its handler bound, so the
            # open question is whether Escape reaches a shell webview at all. The
            # waypointer closes on Escape too, so it answers that without needing
            # a consent request in the frame.
            qmp_key(f, "esc" if args.dismiss_with_escape else "meta_l")
            time.sleep(3)
            capture(f, dismissed, x_display)
            for _ in range(50):
                if os.path.exists(dismissed) and os.path.getsize(dismissed) > 0:
                    break
                time.sleep(0.1)
            # And once more, because the frame being STILL now is not the same as
            # nothing having moved during the measurement. A consent request that
            # lands while the overlay is open changes the screen for its own reasons
            # and the diff below then reports that instead of the overlay. Two
            # post-frames that agree is the evidence that nothing else was in flight.
            confirm = out + ".confirm.png"
            time.sleep(3)
            capture(f, confirm, x_display)
            for _ in range(50):
                if os.path.exists(confirm) and os.path.getsize(confirm) > 0:
                    break
                time.sleep(0.1)
            still_moving = (frame_change(dismissed, confirm)
                            if os.path.exists(confirm) else 1.0)
        if args.deny_consent:
            # Press Escape (the dialog's always-available deny) and capture an
            # after-shot, so the dismissal check confirms the keyboard path reaches
            # the dialog (the main window must grab the keyboard while a request is
            # up) and the shell resolved it against the broker.
            denied = out + ".denied.png"
            qmp_key(f, "esc")
            time.sleep(3)
            capture(f, denied, x_display)
            for _ in range(50):
                if os.path.exists(denied) and os.path.getsize(denied) > 0:
                    break
                time.sleep(0.1)
        if args.click:
            # Coordinates are given against the 1280x800 layout and scaled to the
            # real frame, like the consent click below.
            from PIL import Image
            fw, fh = Image.open(out).size
            for spec in args.click:
                cx, cy = (int(v) for v in spec.split(","))
                qmp_click(f, round(fw * cx / 1280), round(fh * cy / 800), fw, fh)
                time.sleep(1.5)
            clicked = out + ".clicked.png"
            # Long enough for the surface to have ANSWERED, not merely repainted.
            # Two seconds was not: a pane whose read is still in flight shows its
            # empty label, and I read that as "the read returned nothing" and had
            # to withdraw it. A screenshot is only evidence about a load if the
            # load has resolved by the time it is taken.
            time.sleep(args.click_settle)
            capture(f, clicked, x_display)
            for _ in range(50):
                if os.path.exists(clicked) and os.path.getsize(clicked) > 0:
                    break
                time.sleep(0.1)
            print(f"after-click screenshot: {clicked}")
        if args.approve_consent:
            # Click "Allow once" (lower-right of the centered consent card, fixed
            # 1280x800 layout), then capture an after-shot so the dialog-dismissed
            # check can confirm the shell resolved the request against the broker.
            approved = out + ".approved.png"
            from PIL import Image
            fw, fh = Image.open(out).size
            qmp_click(f, round(fw * 797 / 1280), round(fh * 489 / 800), fw, fh)
            time.sleep(3)                  # let the shell poll + hide the resolved dialog
            capture(f, approved, x_display)
            for _ in range(50):
                if os.path.exists(approved) and os.path.getsize(approved) > 0:
                    break
                time.sleep(0.1)
        qmp(f, "quit")
    finally:
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        # The Xvfb only exists to give QEMU somewhere to draw; leaving it running
        # would leak a display server per --gpu run and the next run would pick the
        # same number.
        if xvfb is not None:
            xvfb.terminate()
            try:
                xvfb.wait(timeout=5)
            except subprocess.TimeoutExpired:
                xvfb.kill()

    # Persist the serial BEFORE the screendump-failure exit, so a black or failed
    # boot still saves its log - that is exactly the run whose last init_egl stage
    # marker pins the software-GL hang.
    if args.serial_out and os.path.exists(serial):
        shutil.copyfile(serial, os.path.abspath(args.serial_out))
        print(f"serial: {os.path.abspath(args.serial_out)}")
        # Say how far the capture actually reaches. It does not always reach the
        # end of the run: on 10 Aug a 110s run produced a log whose last entry was
        # at 7.4s, and the dogfood markers land at ~12.7s. Reading that log, their
        # absence looks exactly like the one-shot never emitting, and the wrong
        # conclusion is the natural one. A grep over a truncated log answers a
        # question about the log, not about the system.
        #
        # This reports rather than fails: a genuinely quiet guest also stops
        # logging, so an early last-entry is not proof of truncation. The horizon
        # is what the reader needs either way - it is the line past which absence
        # means nothing.
        try:
            with open(os.path.abspath(args.serial_out), "r", errors="replace") as fh:
                stamps = re.findall(r"^\[\s*(\d+\.\d+)\]", fh.read(), re.M)
        except OSError:
            stamps = []
        if stamps:
            last = float(stamps[-1])
            print(f"serial horizon: last entry at {last:.1f}s of a ~{args.wait}s run"
                  + ("  (nothing after this was captured; absence past it proves nothing)"
                     if last < args.wait * 0.6 else ""))
        else:
            print("serial horizon: no timestamped entries; the log says nothing about timing")

    if not (os.path.exists(out) and os.path.getsize(out) > 0):
        sys.exit("no screenshot captured")
    rendered, summary = inspect(out)
    text = ocr(out)               # whole frame (psm 6) - mainly the console-text guard
    bar_verdict, bar_rgb, bar_detail = top_bar_state(out)
    bar_present = bar_verdict == "present"
    print(f"screenshot: {out} ({summary})")
    if text:
        print("OCR text:\n" + text)
    print(f"top bar: {bar_verdict} (row {bar_rgb}) - {bar_detail}")
    after = out + ".after.png"
    if args.press_super and os.path.exists(after) and os.path.getsize(after) > 0:
        frac = frame_change(out, after)
        verb = "changed the screen" if frac > 0.02 else "had no visible effect"
        print(f"Super press: {verb} ({frac*100:.1f}% of pixels differ) -> {after}")
        if os.path.exists(dismissed) and os.path.getsize(dismissed) > 0:
            # The ghost, stated as a number rather than an impression. `left` is
            # how much of the screen still differs from the pre-open desktop AFTER
            # the overlay was closed: near zero means it was cleaned up, anything
            # like the open-time figure means its last frame is still there.
            left = frame_change(out, dismissed)
            if settled > 0.02 or still_moving > 0.02:
                print(f"Super dismissal: NOT MEASURED - the screen was still "
                      f"changing on its own before the overlay opened "
                      f"({settled*100:.1f}% before, {still_moving*100:.1f}% after, "
                      f"between frames that should have been identical). Give it a "
                      f"longer --wait; a diff against an unsettled baseline says "
                      f"nothing about the overlay.")
            else:
                verdict = ("the overlay's frame is still on screen" if left > 0.02
                           else "the screen returned to the desktop")
                print(f"Super dismissal: {verdict} "
                      f"({left*100:.1f}% still differs, open was {frac*100:.1f}%) -> {dismissed}")
    # A frame full of kernel-console / login text means cosmic-comp never took the
    # scanout (VT/DRM-master conflict) - the getty/console is still on screen, not
    # the compositor. Treat that as failure even though it is "non-black".
    lower = text.lower()
    console_markers = ("login:", "systemd", "audit:", "debian gnu/linux", "kernel")
    if any(m in lower for m in console_markers):
        print("VERIFY FAIL: the frame is the kernel/login console, not the compositor")
        print(f"  serial log: {serial}")
        return 1
    if not rendered:
        print("VERIFY FAIL: frame is blank/black (compositor did not render)")
        print(f"  serial log: {serial}")
        return 1
    if args.require_bar and not bar_present and bar_seen_while_polling:
        # The poll SAW the bar render; the final frame is a later moment, and in a
        # consent run that moment has a fullscreen modal over the desktop by
        # design - `system-dialog-plan.md` wants the consent surface unobscurable,
        # so it covering the bar is the feature working. Failing here turned every
        # `--deny-consent` and `--approve-consent` run red for the wrong reason,
        # and a gate that reports correct behaviour as failure is one people learn
        # to skip.
        print(f"top bar: covered in the final frame ({bar_detail}), but the poll "
              "saw it render earlier - treating the shell as up")
        bar_present = True
    if args.require_bar and not bar_present:
        if bar_verdict == "inconclusive":
            print("VERIFY FAIL: could not tell whether the shell's top bar is on "
                  f"screen - {bar_detail}. This is the gate saying it cannot see, "
                  "not that the shell did not render; look at the frame.")
        else:
            print("VERIFY FAIL: the shell's top bar is absent (compositor up, shell did not render)")
        print(f"  serial log: {serial}")
        return 1
    if args.app:
        # Confirm the SKU launch hook actually fired, not just that --app was passed.
        # Two independent signals, either suffices: (1) the session's explicit marker
        # (arlen-session logs `launching verify app '<app>'`), and (2) the launched
        # app's own journal identifier (`<app>[<pid>]:`). The one-shot session marker
        # is piped through `systemd-cat` and can lose the early journal-to-console
        # forwarding race, whereas the app logs to the journal directly under its own
        # identifier once it is up - which forwards to serial reliably - so its
        # presence is direct launch evidence. A systemd unit line reads `systemd[1]:`,
        # never `<app>[`, so this does not false-match a mere "Started ..." log.
        try:
            with open(serial, "r", errors="replace") as fh:
                journal = fh.read()
        except OSError:
            journal = ""
        if f"launching verify app '{args.app}'" in journal:
            print(f"app: session launched {args.app} (SMBIOS SKU hook fired)")
        elif f"'{args.app}' not an installed binary" in journal:
            print(f"VERIFY FAIL: --app {args.app} - the session saw the SKU but the binary is not installed")
            print(f"  serial log: {serial}")
            return 1
        elif f"{args.app}[" in journal:
            print(f"app: {args.app} is running ({args.app}[pid] in the journal; SKU hook fired)")
        else:
            print(f"VERIFY FAIL: --app {args.app} - no launch signal in the serial "
                  f"(neither the session marker nor a {args.app}[pid] journal line)")
            print(f"  serial log: {serial}")
            return 1
        if args.require_app_text:
            want = args.require_app_text.lower()
            if want not in lower:
                print(f"VERIFY FAIL: --app {args.app} did not show '{args.require_app_text}' "
                      f"(OCR of the frame)")
                print(f"  serial log: {serial}")
                return 1
            print(f"app text: '{args.require_app_text}' present in the frame")
    if args.require_ai:
        try:
            with open(serial, "r", errors="replace") as fh:
                journal = fh.read()
        except OSError:
            journal = ""
        # systemd logs "Started <Description>." per unit; match on each unit's
        # Description. llama-server is a SYSTEM service so its journal reaches the
        # serial reliably and is the hard gate; the AI session daemons are systemd
        # --user services whose logs reach the serial only if the user journal is
        # forwarded, so they are reported but not hard-required (a total AI-layer
        # failure still trips the llama gate, and the dogfood scenario exercises the
        # daemons directly).
        markers = {
            "llama engine": "Arlen local LLM inference engine",
            "audit daemon": "Arlen Audit Daemon",
            "ai proxy": "Arlen AI egress proxy",
            "ai daemon": "Arlen AI daemon",
            "ai agent": "Arlen AI agent",
        }
        present = {k: (v in journal) for k, v in markers.items()}
        print("AI layer: " + ", ".join(
            f"{k}={'up' if p else 'absent'}" for k, p in present.items()))
        if not present["llama engine"]:
            print("VERIFY FAIL: the llama inference engine did not start (no journal marker)")
            print(f"  serial log: {serial}")
            return 1
    if args.require_dogfood:
        try:
            with open(serial, "r", errors="replace") as fh:
                journal = fh.read()
        except OSError:
            journal = ""
        # The dogfood one-shot prints staged markers: EMIT ok (the event reached
        # the bus), ASK ok (a completion came back), then OK; a failure prints
        # DOGFOOD FAIL <reason>. Report the stages, gate on the terminal OK.
        emitted = "DOGFOOD EMIT ok" in journal
        asked = "DOGFOOD ASK ok" in journal
        wrote = "DOGFOOD WRITE ok" in journal
        undid = "DOGFOOD UNDO ok" in journal
        done = "DOGFOOD OK" in journal
        print(f"dogfood: emit={'ok' if emitted else 'absent'}, "
              f"ask={'ok' if asked else 'absent'}, "
              f"write={'ok' if wrote else 'absent'}, "
              f"undo={'ok' if undid else 'absent'}, "
              f"complete={'ok' if done else 'absent'}")
        if not done:
            fail_line = next((ln.strip() for ln in journal.splitlines()
                              if "DOGFOOD FAIL" in ln), None)
            print("VERIFY FAIL: the in-VM KG-AI dogfood did not complete"
                  + (f" ({fail_line})" if fail_line else " (no DOGFOOD OK marker)"))
            print(f"  serial log: {serial}")
            return 1
    if args.require_consent:
        try:
            with open(serial, "r", errors="replace") as fh:
                journal = fh.read()
        except OSError:
            journal = ""
        # The hard gate: the dogfood (a normal attested user app) raised a
        # run_command-shaped ExecConfined request and the broker ACCEPTED it in a
        # RELEASE image. This is the systematic catch for the "works in debug via
        # dev.*, dead in release" admission-bug class - a release boot that refuses
        # the intake never prints this marker.
        raised = "DOGFOOD CONSENT ok" in journal
        skipped = "DOGFOOD CONSENT skipped" in journal
        # The queued-a-dialog broker log (its one-shot intake info line) is the
        # second, independent signal that the request reached the queue.
        queued = "intake: queued for a dialog" in journal
        # Best-effort: did the shell actually RENDER the dialog? OCR of the frame
        # for the request copy. llvmpipe UI-font OCR is unreliable, so this is
        # reported, never gated (the serial markers are the gate).
        # OCR of the thin UI font under llvmpipe misses text that is plainly on
        # screen - it has already reported `absent` for a rendered dialog. So it
        # is never phrased as a verdict: a hit is worth something, a miss says
        # nothing at all, and the wording has to make that impossible to misread.
        ocr_hit = any(s in lower for s in ("sandbox", "run a shell", "uname"))
        print(f"consent: raised={'ok' if raised else 'absent'}, "
              f"queued={'ok' if queued else 'absent'}, "
              f"dialog-ocr={'read the request copy' if ocr_hit else 'no text read (says nothing - llvmpipe OCR misses rendered text)'}"
              + (" (dogfood skipped it)" if skipped else ""))
        if not raised:
            print("VERIFY FAIL: the release consent path is not live "
                  "(no 'DOGFOOD CONSENT ok' - an attested app could not raise an "
                  "intake request in the release image)")
            print(f"  serial log: {serial}")
            return 1
        if args.deny_consent:
            denied = out + ".denied.png"
            if not (os.path.exists(denied) and os.path.getsize(denied) > 0):
                print("VERIFY FAIL: --deny-consent captured no after-Escape frame")
                return 1
            verdict, why = consent_dialog_state(out, denied)
            print(f"consent deny: press Escape -> dialog {verdict} "
                  f"({why}) -> {denied}")
            if verdict == "present":
                # The old wording named the keyboard grab, which is only one of
                # the two ways this fails and not the one measured. See
                # `ghost_or_resolve`.
                print(ghost_or_resolve(serial, "Escape"))
                print(f"  serial log: {serial}")
                return 1
            if verdict == "inconclusive":
                print("VERIFY FAIL: the gate cannot tell whether Escape dismissed "
                      "the dialog. Not a verdict on the build - look at the frame. "
                      "A gate that guesses here is worse than one that stops.")
                print(f"  frame: {denied}")
                return 1
        if args.approve_consent:
            approved = out + ".approved.png"
            if not (os.path.exists(approved) and os.path.getsize(approved) > 0):
                print("VERIFY FAIL: --approve-consent captured no after-click frame")
                return 1
            # Clicking "Allow once" must actually DISMISS the dialog: the click
            # fires allowOnce -> resolve -> the broker removes the request -> the
            # shell's poll clears it. Assert on the dialog's amber bar being GONE,
            # not on a raw frame-diff (the backdrop dimming + the cursor appearing
            # change most of the frame even when the dialog is still up, so a diff
            # threshold false-passes).
            verdict, why = consent_dialog_state(out, approved)
            frac = frame_change(out, approved)
            print(f"consent resolve: click 'Allow once' -> dialog {verdict} "
                  f"({why}; {frac*100:.1f}% of the whole frame changed) "
                  f"-> {approved}")
            if verdict == "present":
                print(ghost_or_resolve(serial, "'Allow once'"))
                print(f"  serial log: {serial}")
                return 1
            if verdict == "inconclusive":
                print("VERIFY FAIL: the gate cannot tell whether 'Allow once' "
                      "dismissed the dialog - an approve that launches something "
                      "repaints the screen the card sat on. Look at the frame; a "
                      "guess either way is a wrong verdict on a real build.")
                print(f"  frame: {approved}")
                return 1
    print("VERIFY OK: " + ("the full desktop rendered (compositor + shell bar)"
                           if bar_present else "the compositor rendered a frame"))
    return 0


if __name__ == "__main__":
    sys.exit(main())

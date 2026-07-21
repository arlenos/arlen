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


def has_top_bar(png):
    """Detect the shell's top bar structurally: it paints a panel strip across the
    top in a distinct colour, so the modal colour of a bar row differs from a
    mid-desktop row. (OCR of the thin UI font under llvmpipe is unreliable, so we
    assert the bar's presence by colour, not text.) Returns (present, bar, desktop)."""
    from PIL import Image
    img = Image.open(png).convert("RGB")
    w, h = img.size

    def modal_row(y):
        row = [img.getpixel((x, y)) for x in range(0, w, 4)]
        return max(set(row), key=row.count)

    bar, desk = modal_row(8), modal_row(h // 2)
    return bar != desk, bar, desk


def consent_dialog_present(png):
    """True if the consent card is on screen, detected by its amber severity bar -
    a distinctive bright-amber strip across the top of the centered card that
    nothing else on the dark desktop paints. Used to confirm a real DISMISSAL after
    a decision click (a raw frame-diff is fooled by the backdrop dimming and the
    cursor appearing, so it cannot tell "resolved" from "still up")."""
    from PIL import Image
    img = Image.open(png).convert("RGB")
    w, h = img.size
    # The card is centered; its top strip sits a little above mid-height. Scan a
    # band there for an amber pixel (high R, mid G, low B).
    y0, y1 = int(h * 0.32), int(h * 0.38)
    x0, x1 = int(w * 0.32), int(w * 0.68)
    for y in range(y0, y1):
        for x in range(x0, x1, 3):
            r, g, b = img.getpixel((x, y))
            if r > 170 and 90 < g < 210 and b < 90 and r > b + 80:
                return True
    return False


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
    ap.add_argument("--require-bar", action="store_true",
                    help="fail unless the shell's top bar is present (full-desktop gate)")
    ap.add_argument("--super", dest="press_super", action="store_true",
                    help="after verifying, press Super and capture a second shot "
                         "(the waypointer/launcher) to exercise the input->shell path")
    ap.add_argument("--app", default=None, metavar="BINARY",
                    help="launch a daily-driver app (its binary name, e.g. "
                         "arlen-system-monitor) in the booted session via QEMU fw_cfg, "
                         "so its window renders for the screenshot (TIER-A 1b). Use a "
                         "longer --wait so the app has time to come up after the shell")
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
        "-vga", "none", "-device", "virtio-gpu-pci",
        # An absolute pointing device so QMP input-send-event abs clicks land
        # (the default q35 mouse is PS/2 relative, which has no fixed origin to
        # click a known pixel). Harmless when no click is driven.
        "-device", "virtio-tablet-pci",
        "-display", "none",
        "-qmp", f"unix:{qmp_path},server,nowait",
        "-serial", f"file:{serial}",
        "-no-reboot",
    ]
    # TIER-A 1b: request the session launch a daily-driver app for the screenshot.
    # The app's binary name rides the SMBIOS system SKU; arlen-session reads it from
    # /sys/class/dmi/id/product_sku (built-in DMI driver, no kernel module) and
    # launches the (sanitised, installed) binary after the shell. Empty SKU on a
    # normal boot, so nothing extra launches.
    if args.app:
        qemu += ["-smbios", f"type=1,sku={args.app}"]
    print("+ " + " ".join(qemu))
    proc = subprocess.Popen(qemu, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        sock, f = qmp_connect(qmp_path, time.monotonic() + 30)
        print(f"QMP connected; letting the session come up ({args.wait}s)...")
        time.sleep(args.wait)
        if args.app:
            # The shell may show a modal (e.g. its consent fixture) over the desktop;
            # press Escape so a launched app window is not hidden behind it.
            qmp_key(f, "esc")
            time.sleep(1.5)
        res = qmp(f, "screendump", filename=out, format="png")
        if "error" in res:
            sys.exit(f"screendump failed: {res['error']}")
        # screendump is async-completed on older QEMU; give it a moment + settle.
        for _ in range(50):
            if os.path.exists(out) and os.path.getsize(out) > 0:
                break
            time.sleep(0.1)
        if args.press_super:
            after = out + ".after.png"
            qmp_key(f, "meta_l")            # Super: the compositor's waypointer toggle
            time.sleep(2)
            qmp(f, "screendump", filename=after, format="png")
            for _ in range(50):
                if os.path.exists(after) and os.path.getsize(after) > 0:
                    break
                time.sleep(0.1)
        if args.deny_consent:
            # Press Escape (the dialog's always-available deny) and capture an
            # after-shot, so the dismissal check confirms the keyboard path reaches
            # the dialog (the main window must grab the keyboard while a request is
            # up) and the shell resolved it against the broker.
            denied = out + ".denied.png"
            qmp_key(f, "esc")
            time.sleep(3)
            qmp(f, "screendump", filename=denied, format="png")
            for _ in range(50):
                if os.path.exists(denied) and os.path.getsize(denied) > 0:
                    break
                time.sleep(0.1)
        if args.approve_consent:
            # Click "Allow once" (lower-right of the centered consent card, fixed
            # 1280x800 layout), then capture an after-shot so the dialog-dismissed
            # check can confirm the shell resolved the request against the broker.
            approved = out + ".approved.png"
            from PIL import Image
            fw, fh = Image.open(out).size
            qmp_click(f, round(fw * 797 / 1280), round(fh * 489 / 800), fw, fh)
            time.sleep(3)                  # let the shell poll + hide the resolved dialog
            qmp(f, "screendump", filename=approved, format="png")
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

    # Persist the serial BEFORE the screendump-failure exit, so a black or failed
    # boot still saves its log - that is exactly the run whose last init_egl stage
    # marker pins the software-GL hang.
    if args.serial_out and os.path.exists(serial):
        shutil.copyfile(serial, os.path.abspath(args.serial_out))
        print(f"serial: {os.path.abspath(args.serial_out)}")

    if not (os.path.exists(out) and os.path.getsize(out) > 0):
        sys.exit("no screenshot captured")
    rendered, summary = inspect(out)
    text = ocr(out)               # whole frame (psm 6) - mainly the console-text guard
    bar_present, bar_rgb, desk_rgb = has_top_bar(out)
    print(f"screenshot: {out} ({summary})")
    if text:
        print("OCR text:\n" + text)
    print(f"top bar: {'present' if bar_present else 'absent'} "
          f"(bar row {bar_rgb}, desktop row {desk_rgb})")
    after = out + ".after.png"
    if args.press_super and os.path.exists(after) and os.path.getsize(after) > 0:
        frac = frame_change(out, after)
        verb = "changed the screen" if frac > 0.02 else "had no visible effect"
        print(f"Super press: {verb} ({frac*100:.1f}% of pixels differ) -> {after}")
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
    if args.require_bar and not bar_present:
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
        dialog_shown = any(s in lower for s in ("sandbox", "run a shell", "uname"))
        print(f"consent: raised={'ok' if raised else 'absent'}, "
              f"queued={'ok' if queued else 'absent'}, "
              f"dialog-ocr={'present' if dialog_shown else 'absent'}"
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
            still_up = consent_dialog_present(denied)
            print(f"consent deny: press Escape -> dialog "
                  f"{'STILL UP' if still_up else 'dismissed'} -> {denied}")
            if still_up:
                print("VERIFY FAIL: Escape did not dismiss the consent dialog (the "
                      "main shell window is not grabbing the keyboard while a "
                      "request is up, so Escape-to-deny never reaches it)")
                print(f"  serial log: {serial}")
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
            still_up = consent_dialog_present(approved)
            frac = frame_change(out, approved)
            print(f"consent resolve: click 'Allow once' -> dialog "
                  f"{'STILL UP' if still_up else 'dismissed'} "
                  f"({frac*100:.1f}% of pixels changed) -> {approved}")
            if still_up:
                print("VERIFY FAIL: the consent dialog did not dismiss after "
                      "'Allow once' (the click did not resolve the request - the "
                      "shell -> broker Resolve leg, or the click landing, is off)")
                print(f"  serial log: {serial}")
                return 1
    print("VERIFY OK: " + ("the full desktop rendered (compositor + shell bar)"
                           if bar_present else "the compositor rendered a frame"))
    return 0


if __name__ == "__main__":
    sys.exit(main())

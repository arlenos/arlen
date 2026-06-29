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
    ap.add_argument("--require-bar", action="store_true",
                    help="fail unless the shell's top bar is present (full-desktop gate)")
    ap.add_argument("--super", dest="press_super", action="store_true",
                    help="after verifying, press Super and capture a second shot "
                         "(the waypointer/launcher) to exercise the input->shell path")
    ap.add_argument("--require-ai", action="store_true",
                    help="fail unless the AI layer came up: the journal (forwarded to "
                         "serial) must show the llama engine + the AI session daemons started")
    ap.add_argument("--require-dogfood", action="store_true",
                    help="fail unless the in-VM KG-AI dogfood completed: the serial "
                         "journal must show 'DOGFOOD OK' (event injected + AI completion). "
                         "Implies the AI layer; use a longer --wait (the loop waits a "
                         "promotion pass then asks, with retry for model-load latency)")
    args = ap.parse_args()

    image = os.path.abspath(args.image)
    if not os.path.exists(image):
        sys.exit(f"image not found: {image} (run dev/mkosi/build-image.sh first)")

    tmp = tempfile.mkdtemp(prefix="arlen-verify-")
    vars_fd = os.path.join(tmp, "OVMF_VARS.fd")
    shutil.copyfile(OVMF_VARS, vars_fd)
    qmp_path = os.path.join(tmp, "qmp.sock")
    serial = os.path.join(tmp, "serial.log")
    out = os.path.abspath(args.out)

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
        "-drive", f"if=virtio,format=raw,file={image}",
        # single virtio-gpu, no default VGA: cosmic-comp gets one DRM device with a
        # render node + GBM, and screendump captures that scanout. No gl=on, so the
        # framebuffer is CPU-readable (llvmpipe does the GL).
        "-vga", "none", "-device", "virtio-gpu-pci",
        "-display", "none",
        "-qmp", f"unix:{qmp_path},server,nowait",
        "-serial", f"file:{serial}",
        "-no-reboot",
    ]
    print("+ " + " ".join(qemu))
    proc = subprocess.Popen(qemu, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        sock, f = qmp_connect(qmp_path, time.monotonic() + 30)
        print(f"QMP connected; letting the session come up ({args.wait}s)...")
        time.sleep(args.wait)
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
        qmp(f, "quit")
    finally:
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()

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
    print("VERIFY OK: " + ("the full desktop rendered (compositor + shell bar)"
                           if bar_present else "the compositor rendered a frame"))
    return 0


if __name__ == "__main__":
    sys.exit(main())

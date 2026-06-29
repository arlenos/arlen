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


def main():
    ap = argparse.ArgumentParser()
    here = os.path.dirname(os.path.abspath(__file__))
    ap.add_argument("--image", default=os.path.join(here, "..", "mkosi", "arlen.raw"))
    ap.add_argument("--wait", type=int, default=40, help="seconds to let the session come up")
    ap.add_argument("--out", default=os.path.join(here, "shot.png"))
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
        "qemu-system-x86_64", "-machine", "q35,accel=kvm:tcg", "-m", "2048", "-smp", "2",
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
        qmp(f, "quit")
    finally:
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()

    if not (os.path.exists(out) and os.path.getsize(out) > 0):
        sys.exit("no screenshot captured")
    rendered, summary = inspect(out)
    text = ""
    try:
        text = subprocess.run(["tesseract", out, "-", "--psm", "6"],
                              capture_output=True, text=True, timeout=30).stdout.strip()
    except Exception:
        pass
    print(f"screenshot: {out} ({summary})")
    if text:
        print("OCR text:\n" + text)
    # A frame full of kernel-console / login text means cosmic-comp never took the
    # scanout (VT/DRM-master conflict) - the getty/console is still on screen, not
    # the compositor. Treat that as failure even though it is "non-black".
    lower = text.lower()
    console_markers = ("login:", "systemd", "audit:", "debian gnu/linux", "kernel")
    if any(m in lower for m in console_markers):
        print("VERIFY FAIL: the frame is the kernel/login console, not the compositor")
        print(f"  serial log: {serial}")
        return 1
    if rendered:
        print("VERIFY OK: the compositor rendered a frame")
        return 0
    print("VERIFY FAIL: frame is blank/black (compositor did not render)")
    print(f"  serial log: {serial}")
    return 1


if __name__ == "__main__":
    sys.exit(main())

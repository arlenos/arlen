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

# How long after boot the knowledge probe has a verdict. The probe asks twice so it
# can tell "nothing yet" from "nothing ever"; only the second round is a verdict,
# so a run that ends before it has no probe result to read.
#
# The second round lands at ~82s (5s to start, then its 75s gap) and since 18
# August it then POLLS the ingestion question for up to 90s more, because with the
# kernel sensor forwarding, the dogfood's own file reaches the graph later than the
# round does. A run that ends at 90s now cuts the probe off mid-poll and reads as
# an ingestion failure that is really a harness that did not wait - the same shape
# as the 13 Aug fault this constant was introduced for, one layer along.
PROBE_ROUND_GAP = 180

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


class QmpError(RuntimeError):
    """QEMU refused a command.

    Raised rather than returned because every caller here ignored the reply, so
    a refusal read exactly like success: a mistyped qcode (`supr` for `esc`) or
    a coordinate outside the screen would be reported by this script as a key
    pressed and a click driven, and the run would then measure a machine nobody
    had touched. An instrument that says it did something it did not do is worse
    than one that cannot do it at all.
    """


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
        if "error" in msg:
            err = msg["error"]
            raise QmpError(
                f"{execute}: {err.get('class', 'error')}: {err.get('desc', err)}"
            )
        if "return" in msg:
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


def timed_plan(shot_at, click_at, key_at):
    """The timed frames, clicks and keys as one list in time order.

    Separate from the loop that runs it so the ordering can be tested without a
    VM, because the ordering is the whole point: a click aimed INTO the gap
    between two frames is only evidence if it happens between them, and the
    click's own probe frame is what says what was on screen when it landed.

    Raises ValueError on a spec it cannot read, rather than skipping it - a
    mistyped `--click-at` that silently did nothing would turn a real finding
    into a quiet pass.
    """
    plan: list[tuple[float, str, object]] = []
    for at in shot_at or []:
        plan.append((float(at), "shot", None))
    for spec in click_at or []:
        when, sep, where = spec.partition(":")
        if not sep or "," not in where:
            raise ValueError(f"--click-at wants SECONDS:X,Y, got {spec!r}")
        cx, cy = (int(v) for v in where.split(","))
        plan.append((float(when), "click", (cx, cy)))
    for spec in key_at or []:
        when, sep, name = spec.partition(":")
        if not sep or not name:
            raise ValueError(f"--key-at wants SECONDS:NAME, got {spec!r}")
        plan.append((float(when), "key", name))
    # Stable within one instant: a shot before a click at the same second says
    # what the click was aimed at.
    order = {"shot": 0, "key": 1, "click": 2}
    return sorted(plan, key=lambda t: (t[0], order[t[1]]))


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
    except FileNotFoundError:
        # NONE, NOT "". The two are different answers and this returned the same
        # one for both until 20 August: on a host with no tesseract every OCR
        # read came back empty, which made `--require-app-text` fail for an app
        # that had drawn perfectly AND made the console-text guard - which fails
        # a boot where the getty is still on screen - pass by finding no markers
        # in a string nothing had looked at. Wrong in both directions, silently.
        # Callers must decide what to do with "I could not look".
        return None
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


def _blank_pointer(img, at, w, h):
    """Paint out the mouse pointer before measuring contrast.

    The pointer is drawn by the compositor's cursor plane, not by the surface
    under test, so it has no business in a measurement about that surface. It was
    in one: the approve path clicks 'Allow once' at the middle of the screen,
    which is inside the sampled card box by construction, and a white arrow on a
    dark desktop reads as contrast. Measured on 10 August - the approved frame's
    card box scored 241 against a desktop 3, and blanking a 45px patch at the
    click point dropped it to 4. Two runs were called "the card is still on
    screen, this is PR-20" against a frame that shows a bare desktop.

    That is also why the verdict lined up so neatly with allowed-versus-denied:
    approve leaves the pointer in the box, Escape leaves it wherever it was.
    """
    if at is None:
        return img
    cx, cy = at
    patch = img.copy()
    # A pointer's hotspot is its TIP, so the glyph hangs DOWN and RIGHT of the
    # point that was clicked; a box centred on that point clips its tail. The test
    # caught exactly that - a symmetric 25px patch left the arrow's lower tip in
    # frame and the spread stayed at 225. So reach much further down-right than up-
    # left, and keep a small margin for the outline and any drop shadow.
    back = max(12, int(w * 0.01))
    fwd = max(48, int(w * 0.045))
    fill = img.getpixel((max(0, min(w - 1, cx + 4 * fwd)), max(0, min(h - 1, cy))))
    for x in range(max(0, cx - back), min(w, cx + fwd)):
        for y in range(max(0, cy - back), min(h, cy + fwd)):
            patch.putpixel((x, y), fill)
    return patch


def consent_card_box(png):
    """Where the consent card is in this frame, or `None` if it is not there.

    FOUND, NOT REMEMBERED. The approve click used to go to a coordinate computed
    from the frame size on the assumption of a fixed 1280x800 layout. That is a
    guess about where a window will be, and on 20 August it guessed onto the
    screenshot app's floating thumbnail: the run captured a screen, the thumbnail
    appeared under the remembered point, and the click dismissed it - which
    auto-saves a file. Nothing was harmed on a VM whose disk is discarded, but a
    harness that clicks blind will eventually press something that matters.

    The card is located by its own top border, which is the one warm strip on an
    otherwise dark desktop, and the bottom by following the card's centre column
    down until it returns to the desktop colour.
    """
    from PIL import Image

    img = Image.open(png).convert("RGB")
    w, h = img.size
    px = img.load()
    warm = lambda c: c[0] > 180 and c[1] > 120 and c[2] < 90
    top = next(
        (y for y in range(h) if sum(1 for x in range(0, w, 4) if warm(px[x, y])) > 20),
        None,
    )
    if top is None:
        return None
    xs = [x for x in range(w) if warm(px[x, top])]
    left, right = min(xs), max(xs)
    centre = (left + right) // 2
    desktop = px[10, h // 2]
    bottom = top
    for y in range(top, h):
        c = px[centre, y]
        if sum(abs(a - b) for a, b in zip(c, desktop)) > 12:
            bottom = y
    return (left, top, right, bottom)


#: Where "Allow once" sits inside the card, as a fraction of the card's own box.
#: Measured off a real frame rather than assumed: the button centre at (797, 489)
#: in a card spanning x 418-861, y 272-529.
ALLOW_ONCE_IN_CARD = (0.856, 0.844)


def consent_dialog_state(before_png, after_png, pointer_at=None):
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

    after = _blank_pointer(after, pointer_at, w, h)

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


def window_top(png):
    """The y of the app window's top edge in FRAME pixels, or None.

    A `--click X,Y` is a bet that the window is where it was last time, and on
    21 August that bet lost twice: the same click that had opened `spec.pdf` on
    two boots opened the `work` folder on two others, because the window sat 33
    pixels lower and the rows are 32 pixels apart. Nobody positions that window
    and nothing promises it will land twice in the same place, so a coordinate
    measured off one screenshot is only good for the screenshot it came from.

    This finds the edge instead. The desktop under an app is one flat colour, so
    a column down the middle of the screen leaves it exactly once: at the top of
    the window. Four rows in a row must differ, which steps over the soft shadow
    and over a stray pointer pixel.

    Returns None rather than a guess when the column never leaves the desktop -
    no window, or a window that fills the screen - and the caller refuses.
    """
    from PIL import Image

    im = Image.open(png).convert("RGB")
    w, h = im.size
    px = im.load()
    # The left margin, well below the bar: desktop in every placement seen so far.
    ref = px[int(w * 0.02), int(h * 0.5)]
    col = int(w * 0.5)
    run = 0
    for y in range(int(h * 0.06), h):
        if max(abs(a - b) for a, b in zip(px[col, y], ref)) > 25:
            run += 1
            if run >= 4:
                return y - 3
        else:
            run = 0
    return None


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


# The identity events that mean the chain is inconsistent with ITSELF, as opposed
# to unavailable. Keyed on the audit event names the SDK emits, so a rename in the
# resolver shows up here as a check that stops matching rather than as silence.
IDENTITY_FAULTS = (
    ("identity.divergence", "two resolvers named one process differently"),
    (
        "identity.broker_returned_reserved_or_invalid",
        "a reader threw away a stamp the registrar was allowed to make",
    ),
)


def identity_faults(journal_text):
    """The identity faults present in `journal_text`, as printable lines."""
    out = []
    for event, meaning in IDENTITY_FAULTS:
        hits = [ln for ln in journal_text.splitlines() if event in ln]
        if hits:
            out.append(f"{event} x{len(hits)}: {meaning}")
            out.append(f"    {hits[0].strip()[:200]}")
    return out


# Where the knowledge daemon's event store lands in the guest, most likely first.
#
# `pick_data_path` (daemons/knowledge/src/utils.rs) prefers `ARLEN_DB_PATH`, then
# the per-user directory when HOME is set, then the system default - and the
# desktop runs the daemon as a user service, so the per-user path is the one a
# boot actually produces. Both are searched because the system path is still what
# a root-run daemon would write, and a check that guessed one and reported "no
# store" for the other would be a false alarm about the most fundamental thing.
EVENT_STORE_PATHS = (
    "/home/*/.local/share/arlen/events.db",
    "/var/lib/arlen/knowledge/events.db",
)


# What the journal calls a daemon, where that differs from the binary the socket
# table names. These are the `systemd-cat --identifier=` tags the session chooses
# for the children it spawns, so they are ours and short by design.
JOURNAL_ALIASES = {
    "arlen-shell": "arlen-desktop-shell",
    "arlen-supervisor": "arlen-session-supervisor",
    "arlen-compositor": "arlen-compositor",
}


def observed_servers(text, also=()):
    """(socket name, serving binary) pairs a run actually showed binding.

    `also` names servers that do not look like ours. The pattern recognises
    `arlen-*` and `event-bus`, which covered 28 of the 29 table entries and
    silently skipped the 29th: `xdg-desktop-portal-arlen` serves portal-picker.sock
    and is named for the freedesktop interface it implements, not for us. A wrong
    table entry for it could never have been caught by a boot - the one daemon the
    reality check could not see was invisible for a reason that had nothing to do
    with it.

    Callers pass the table's own values, so a daemon added to the table becomes
    observable with it and there is no second list to keep. Foreign names stay out
    deliberately: matching any process that binds a `/run` socket would report
    systemd and dbus as unnamed servers on every boot.
    """
    names = "|".join(
        ["arlen-[a-z0-9-]+", "event-bus", *(re.escape(n) for n in sorted(set(also)))]
    )
    seen = set()
    for line in text.splitlines():
        who = re.search(rf"({names})\[\d+\]", line)
        socks = re.findall(r"(/run[^\s\"]*\.sock)", line)
        if not who or not socks:
            continue
        if not re.search(r"listen|bound|serving", line, re.I):
            continue
        name = JOURNAL_ALIASES.get(who.group(1), who.group(1))
        for sock in socks:
            seen.add((sock.rsplit("/", 1)[-1], name))
    return seen


def socket_table_faults(text):
    """Where the hand-kept socket table disagrees with what the boot did.

    The table is drift-checked in CI against the source; this is the other half -
    the run itself says which binary bound which socket, so a wrong VALUE is caught
    by reality rather than by review. Only what the boot SHOWED is judged: a table
    entry for a daemon this image does not start is not evidence of anything.
    """
    servers = _socket_table()
    if servers is None:
        return ["could not read the socket table, so the boot verified nothing about it"]
    out = []
    for sock, who in sorted(observed_servers(text, servers.values())):
        expected = servers.get(sock)
        if expected is None:
            out.append(f"{sock}: bound by {who} on this boot and named by no table entry")
        elif expected != who:
            out.append(f"{sock}: the table says {expected} serves it, the boot showed {who}")
    return out


def _socket_table():
    """The SERVERS dict from the CI gate, or None when it cannot be read.

    Loaded by path rather than imported: the file's name has a hyphen, so it is not
    a module name, and copying the table here would be the drift both checks exist
    to prevent.
    """
    import importlib.util

    path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                        "..", "scripts", "check-socket-servers.py")
    if not os.path.exists(path):
        return None
    spec = importlib.util.spec_from_file_location("socket_servers_table", path)
    module = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(module)
    except SystemExit:
        pass
    except Exception:
        return None
    return getattr(module, "SERVERS", None)


def probe_shipped_so_far(serial_path):
    """Whether the knowledge probe has started, judged from the serial so far.

    Same question `probe_is_shipped` answers after the run, asked DURING it so the
    run can stay alive long enough to have an answer to read. Deliberately the same
    predicate on the same text rather than a second rule about unit names: two
    rules for one fact drift, and the drift here is silent - the verdict arms, the
    wait does not, and every run fails for a harness reason.
    """
    from probe_verdict import probe_is_shipped

    if not serial_path or not os.path.exists(serial_path):
        return False
    with open(serial_path, "r", errors="replace") as fh:
        return probe_is_shipped(fh.read())


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


def another_card_was_raised(serial_path):
    """Whether the shell raised a NEW consent card after taking the last one down.

    The third answer this gate did not have. A card on screen after a resolve is
    a stale frame OR a failed resolve - unless the queue had another request in
    it, in which case the shell answered the first, hid the window, and raised it
    again for the next one. That frame is CORRECT and both existing verdicts
    misread it: one sends the reader to PR-20, the other blames the resolve leg.

    Measured on the 22 August image, in this order and within a second:

        consent_resolve: id=0 outcome=AllowedOnce
        consent: card for 0 is out of the DOM (0 dialog node(s) remain)
        consent_window::hide: dropped
        consent_window::show: raised          <- the next request

    So a `show` after the last `hide` is the tell.
    """
    if not serial_path or not os.path.exists(serial_path):
        return False
    try:
        with open(serial_path, "r", errors="replace") as fh:
            text = fh.read()
    except OSError:
        return False
    hide = text.rfind("consent_window::hide")
    show = text.rfind("consent_window::show: raised")
    return hide >= 0 and show > hide


def ghost_or_resolve(serial_path, how):
    """The failure line for a card that is still on screen after `how`."""
    gone = card_left_the_dom(serial_path)
    if gone and another_card_was_raised(serial_path):
        # Not a failure at all: answered, taken down, and the next one in the
        # queue put up. The dogfood loop asks about once a minute, so this is the
        # ORDINARY shape of a run that waits ninety seconds and then clicks.
        return None
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
    ap.add_argument("--super-before-click", dest="super_before_click",
                    action="store_true",
                    help="press Super just before the clicks, so the launcher can"
                         " be driven AFTER a consent card has been answered")
    ap.add_argument("--super-settle", type=float, default=3.0,
                    help="seconds to wait after --super-before-click")
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
    ap.add_argument("--click-in-window", dest="click_in_window", action="append",
                    default=None, metavar="X,DY",
                    help="click X (screen) at DY BELOW the app window's top edge, "
                         "found in the pre-click frame. Use this for anything "
                         "inside a window: the placement moves between boots and "
                         "a fixed --click lands on the wrong row when it does.")
    ap.add_argument("--key", action="append", default=None, metavar="NAME",
                    help="a QMP key name pressed after the clicks, e.g. `ret` to open"
                         " the row a click just selected")
    ap.add_argument("--click-settle", type=int, default=8, metavar="SECONDS",
                    help="with --click, how long to wait before the after-shot. The "
                         "default is generous on purpose: the shot is evidence about "
                         "what the surface ANSWERED, and a pane still loading looks "
                         "exactly like a pane that answered nothing")
    ap.add_argument("--require-app-text", default=None, metavar="SUBSTR",
                    help="with --app, fail unless the screenshot OCRs a substring "
                         "(case-insensitive), e.g. a process name the app must show")
    ap.add_argument("--require-ai", action="store_true",
                    help="fail unless the llama inference engine started (a system "
                         "unit, so its journal reaches the serial reliably). The AI "
                         "session daemons are user units whose logs only reach the "
                         "serial if the user journal is forwarded, so they are "
                         "REPORTED, not required - this said they 'must show' for "
                         "weeks while gating on the engine alone")
    ap.add_argument("--require-dogfood", action="store_true",
                    help="fail unless the in-VM KG-AI dogfood completed: the serial "
                         "journal must show an injected event and a terminal marker. "
                         "The AI completion leg is best-effort and is REPORTED, not "
                         "required - it said 'event injected + AI completion' here "
                         "for weeks while gating on neither. "
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
    ap.add_argument("--require-graph", action="store_true",
                    help="fail unless the GRAPH STORE itself, read on the host with "
                         "our own engine, holds this boot's file and its project "
                         "link. Off by default because the graph leg needs promotion "
                         "to have run and a project to link to, so a sound boot can "
                         "lack it; the answer is printed either way. Everything else "
                         "about the KG grades what the guest said about itself")
    ap.add_argument("--require-kernel", action="store_true",
                    help="fail unless the EVENT STORE holds at least one event from "
                         "the eBPF sensor. Off by default for the same reason as "
                         "--require-graph: the sensor's program is loaded at runtime "
                         "and the verifier can refuse it, which is exactly the thing "
                         "worth seeing, so the answer is printed on every run and "
                         "this is the switch for a run that means to hold it")
    ap.add_argument("--journal-out", default=None, metavar="PATH",
                    help="write the guest's own journal here, read out of the "
                         "overlay after it halts. Unlike the serial log this covers "
                         "the whole run.")
    ap.add_argument("--require-probe", action="store_true",
                    help="fail unless the knowledge probe answered every question "
                         "AND found something. Needs --linger past ~172s - the "
                         "second round at 82s plus its 90s ingestion poll - and is "
                         "raised to that automatically. Implied whenever the "
                         "serial shows the probe unit starting, so an image that "
                         "ships the probe is held to it without being asked; pass "
                         "this to demand it from an image that may not.")
    ap.add_argument("--inspect", action="append", default=None, metavar="/PATH",
                    help="after the run, list this directory out of the overlay and "
                         "print what is in it. For the question a frame cannot "
                         "answer: the window said it saved a file, so is the file "
                         "there. Repeatable.")
    ap.add_argument("--preload", action="append", default=None, metavar="SRC:/DEST[:MODE]",
                    help="copy a host file into the guest before boot, into the "
                         "throwaway overlay rather than the image. For state that "
                         "has to exist BEFORE the session starts - a calendar file "
                         "whose alarm the daemon reads at startup, a config a "
                         "first-run path branches on. Repeatable. MODE is octal "
                         "(0755) for a file that has to be executable - the copy "
                         "does not carry the source's own bit.")
    ap.add_argument(
        "--key-at",
        action="append",
        metavar="SECONDS:NAME",
        help="press a QMP key this many seconds after QMP connects, on the same "
        "clock as --shot-at and --click-at. Answering a queued dialog is often the "
        "PRECONDITION for the thing being measured - a broker that shows one card "
        "at a time never raises a second one until the first is answered.",
    )
    ap.add_argument(
        "--click-at",
        action="append",
        metavar="SECONDS:X,Y",
        help="click these 1280x800 coordinates this many seconds after QMP connects, "
        "on the same clock as --shot-at. For driving input INTO a gap - a surface "
        "that holds the desktop's input before it has painted can be asked the "
        "question by clicking where its button will be, and whether the answer is "
        "taken is the whole finding.",
    )
    ap.add_argument(
        "--shot-at",
        type=float,
        action="append",
        metavar="SECONDS",
        help="capture an extra frame this many seconds after QMP connects, written "
        "beside --out as `<out>.t<SECONDS>.png`. Repeatable. For a question about "
        "WHEN something appears rather than whether it did: a surface that is raised "
        "at 14s and shows its first pixels seconds later cannot be told from one that "
        "renders instantly by a single shot at the end.",
    )
    ap.add_argument("--linger", type=int, default=0, metavar="SECONDS",
                    help="stay alive this long after the checks pass, before the "
                         "shutdown. Pair with --keep to get a journal that covers "
                         "steady state and not only startup.")
    ap.add_argument("--shutdown-wait", type=int, default=30, metavar="SECONDS",
                    help="how long to let the guest halt after the ACPI powerdown "
                         "before pulling the plug. Only the journal depends on it, "
                         "so a small number costs a short log and never a verdict.")
    ap.add_argument("--keep", action="store_true",
                    help="keep the workdir even when the run passes. The overlay "
                         "holds the guest's persistent journal, which is the only "
                         "place the whole boot exists: the serial console stops "
                         "carrying userspace output around 7s while the guest keeps "
                         "running (measured 12 Aug across three boots). A passing "
                         "run is exactly when you want to go read that.")
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
    # And which tree it came from, when the build left a stamp beside it. An image
    # is an opaque 5 GB file whose mtime against `git log` is a guess; this is the
    # build saying so itself. Absent for images built before 18 August, and absence
    # is silent because an old image is not a fault.
    stamp = image + ".stamp"
    if os.path.exists(stamp):
        for line in open(stamp).read().splitlines():
            print(f"  {line}", flush=True)
    if not os.path.exists(image):
        sys.exit(f"image not found: {image} (run dev/mkosi/build-image.sh first)")

    # Removed on a clean run, KEPT on a failing one. It holds the boot's OVMF
    # vars, its qcow2 overlay and the raw serial log, which are the things you
    # want when a run fails and useless when it passes - so the rule is the same
    # one the image build's cleanup arrived at: throw away what nobody needs,
    # never what a failure would be diagnosed from.
    #
    # It leaked before this: every run since the harness was written left one
    # behind, 56 of them holding 1.3G by 11 Aug. That matters beyond tidiness -
    # a full disk is what killed a build mid-write and left a half-image that
    # verified as a broken system for an hour.
    tmp = tempfile.mkdtemp(prefix="arlen-verify-")
    # Said out loud because the failure paths leave it behind on purpose, and a
    # kept directory nobody can find is the same as a deleted one.
    print(f"workdir: {tmp} (kept if this run fails)", flush=True)
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

    # Files the run wants in the guest before anything boots.
    #
    # WHY THIS EXISTS. Some things can only be verified against state that is
    # already there when the session starts: a reminder rings because a calendar
    # file with an alarm in it was on disk before the daemon read it, and there is
    # no clicking your way to "this existed at boot". Writing into the OVERLAY keeps
    # the promise the overlay makes - the raw image stays untouched and the next run
    # starts pristine.
    if args.preload:
        pairs = []
        for spec in args.preload:
            src, _, rest = spec.partition(":")
            dest, _, mode = rest.partition(":")
            if not src or not dest or not dest.startswith("/"):
                print(f"VERIFY FAIL: --preload wants SRC:/absolute/dest[:MODE], got {spec!r}", file=sys.stderr)
                return 2
            if not os.path.isfile(src):
                print(f"VERIFY FAIL: --preload source {src} is not a file", file=sys.stderr)
                return 2
            if mode and not re.fullmatch(r"0?[0-7]{3}", mode):
                print(f"VERIFY FAIL: --preload mode must be octal like 0755, got {mode!r}", file=sys.stderr)
                return 2
            pairs.append((src, dest, mode))
        script = ["run", "mount /dev/sda2 /"]
        for src, dest, mode in pairs:
            script.append(f"mkdir-p {os.path.dirname(dest)}")
            script.append(f"upload {src} {dest}")
            # `upload` does not carry the source's mode, so a preloaded script
            # lands unexecutable and the thing that runs it dies without saying
            # why: greetd opened a session and closed it 20ms later, and the run
            # still reported OK. Naming the mode is how you preload something
            # meant to be RUN rather than read.
            if mode:
                script.append(f"chmod 0{int(mode, 8):o} {dest}")
        # `guestfish` over the qcow2 overlay, so the writes land in the throwaway.
        r = subprocess.run(
            ["guestfish", "--rw", "-a", overlay],
            input="\n".join(script) + "\n",
            text=True,
            capture_output=True,
        )
        if r.returncode != 0:
            print(f"VERIFY FAIL: could not preload into the overlay: {r.stderr.strip()}", file=sys.stderr)
            return 2
        for _, dest, mode in pairs:
            print(f"preloaded {dest}" + (f" (mode 0{int(mode, 8):o})" if mode else ""))

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
    run_start = time.monotonic()
    proc = subprocess.Popen(qemu, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, env=qemu_env)
    try:
        sock, f = qmp_connect(qmp_path, time.monotonic() + 30)
        t_qmp = time.monotonic()
        print(f"QMP connected; letting the session come up ({args.wait}s)...")
        # For the pure top-bar gate (the black-screen rate measurement), POLL for the
        # bar rather than a single wait-then-shot. Under heavy load the shell's WebKit
        # render can finish just after a fixed wait, which one shot mis-reports as
        # black; polling reports "did the bar render WITHIN N seconds" and the elapsed
        # time is the true time-to-render. The app/consent/super modes keep the fixed
        # wait their after-steps depend on.
        # Set when the poll actually saw the bar, so a modal covering the final
        # frame cannot erase that evidence.
        # The extra frames and the timed clicks share one clock and one ordering,
        # so a click can be aimed INTO the gap between two frames.
        for at, kind, arg in timed_plan(args.shot_at, args.click_at, args.key_at):
            left = at - (time.monotonic() - t_qmp)
            if left > 0:
                time.sleep(left)
            if kind == "key":
                qmp_key(f, arg)
                print(f"key at {at:g}s: {arg}")
                continue
            if kind == "click":
                cx, cy = arg
                # The frame size is not known before the first capture, and a
                # click needs it. Take one to learn it; it is thrown away.
                probe = f"{out}.click{at:g}.png"
                capture(f, probe, x_display)
                for _ in range(50):
                    if os.path.exists(probe) and os.path.getsize(probe) > 0:
                        break
                    time.sleep(0.1)
                try:
                    cw, ch = Image.open(probe).size
                except Exception:
                    cw, ch = 1280, 800
                qmp_click(f, round(cw * cx / 1280), round(ch * cy / 800), cw, ch)
                print(f"click at {at:g}s: {cx},{cy}")
                continue
            frame = f"{out}.t{at:g}.png"
            shot = capture(f, frame, x_display)
            if "error" in shot:
                print(f"shot at {at:g}s: {shot['error'].get('desc', shot['error'])}")
            else:
                for _ in range(50):
                    if os.path.exists(frame) and os.path.getsize(frame) > 0:
                        break
                    time.sleep(0.1)
                print(f"shot at {at:g}s: {frame}")

        bar_seen_while_polling = False
        # Set only when a modal was dismissed and a fresh shot taken; see the note
        # at the app-text check for why the two frames are not interchangeable.
        uncovered_frame = None
        bar_gate_only = args.require_bar and not (
            args.app or args.press_super or args.deny_consent or args.approve_consent
            or args.click or args.click_in_window
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
        if args.approve_consent:
            # Click "Allow once" (lower-right of the centered consent card, fixed
            # 1280x800 layout), then capture an after-shot so the dialog-dismissed
            # check can confirm the shell resolved the request against the broker.
            approved = out + ".approved.png"
            uncovered_frame = approved
            from PIL import Image
            fw, fh = Image.open(out).size
            # A FRESH frame to look at, not `out`. That one was captured earlier
            # in the run and the card may have been raised, covered or repainted
            # since - the first version of this reused it and refused to click on
            # a run where the card was up, because the older frame did not have
            # it. What we need is where the card is NOW.
            locate = out + ".locate.png"
            capture(f, locate, x_display)
            for _ in range(50):
                if os.path.exists(locate) and os.path.getsize(locate) > 0:
                    break
                time.sleep(0.1)
            box = consent_card_box(locate) or consent_card_box(out)
            if box is None:
                # Refuse rather than click somewhere. There is no card in this
                # frame, so any coordinate is a guess about somebody else's
                # window - which is how this pressed an app's own button once.
                print("VERIFY FAIL: --approve-consent found no consent card in the frame, "
                      "so there was nothing to approve and nothing was clicked")
                print(f"  frame: {out}")
                return 1
            left, top, right, bottom = box
            cx = round(left + (right - left) * ALLOW_ONCE_IN_CARD[0])
            cy = round(top + (bottom - top) * ALLOW_ONCE_IN_CARD[1])
            print(f"consent card at {box}; clicking 'Allow once' at ({cx}, {cy})")
            qmp_click(f, cx, cy, fw, fh)
            time.sleep(3)                  # let the shell poll + hide the resolved dialog
            capture(f, approved, x_display)
            for _ in range(50):
                if os.path.exists(approved) and os.path.getsize(approved) > 0:
                    break
                time.sleep(0.1)
        # Let the guest live a while before shutting it down, when asked.
        #
        # The bar-polling modes return the moment the bar appears, which is the right
        # behaviour for a pass/fail gate and means a run is over at about 11s. With
        # the journal now complete, that completeness buys nothing on its own: a whole
        # record of the first nine seconds is still a record of startup. Lingering is
        # what turns this into "and then it behaved" - a daemon retrying, a socket
        # refused on the tenth attempt, a timer firing - all of which happen in the
        # part no run has ever stayed alive for.
        # The probe's answer arrives on its SECOND round, 75s after the first, and
        # the verdict below self-arms the moment an image ships the probe unit. So
        # the wait has to self-arm too, or every default run on a probe-carrying
        # image fails for a reason that is about the harness rather than the image -
        # and a failure that is always there is one an operator learns to ignore,
        # which is worse than not checking at all. Walked into exactly that on the
        # 13 Aug run: VERIFY FAIL, nothing wrong with the image.
        linger = args.linger
        # CONSENT FIRST, THEN CLICKS. These were the other way round, and the
        # order is not cosmetic: the dogfood consent card covers the middle of the
        # screen, which is exactly where an app window is, so a run given both
        # flags aimed its clicks at whatever the card happened to be showing. A
        # click that lands on a dialog nobody meant to answer is the same failure
        # as the blind approve click that once dismissed the screenshot app's
        # thumbnail. Resolving the card first means the clicks land on the app.
        if args.click or args.click_in_window:
            # Coordinates are given against the 1280x800 layout and scaled to the
            # real frame, like the consent click below.
            from PIL import Image
            fw, fh = Image.open(out).size
            # A frame from immediately before the clicks, because the main
            # screenshot is not a usable baseline for them. Measured on 10 August:
            # a run that clicked a top-bar indicator had a consent card covering
            # the screen at main-capture time, so both frames were about the
            # consent surface and the diff between them read as 99.995% - which
            # says nothing whatever about the popover that was the subject. This
            # capture happens after the consent flow above has resolved, so it is
            # the state the clicks actually start from.
            # Super is pressed HERE, not in the --super block near the top, when
            # a run needs the launcher AFTER the consent card is answered. The
            # blocks run in source order (super, then consent, then clicks), so
            # `--super --click` opens the launcher and the card then covers it:
            # on 21 August a run meant to click a launcher row photographed the
            # consent card instead, because the launcher had already gone. A
            # first boot answers the card and THEN uses the desktop, so this is
            # the ordinary sequence and not a special case.
            if args.super_before_click:
                qmp_key(f, "meta_l")
                time.sleep(args.super_settle)
            preclick = out + ".preclick.png"
            capture(f, preclick, x_display)
            for _ in range(50):
                if os.path.exists(preclick) and os.path.getsize(preclick) > 0:
                    break
                time.sleep(0.1)
            for spec in args.click or []:
                cx, cy = (int(v) for v in spec.split(","))
                qmp_click(f, round(fw * cx / 1280), round(fh * cy / 800), fw, fh)
                time.sleep(1.5)
            if args.click_in_window:
                top = window_top(preclick)
                if top is None:
                    sys.exit("--click-in-window: no window edge in the pre-click "
                             "frame, so there was nothing to measure from and "
                             "nothing was clicked (see %s)" % preclick)
                print(f"window top at y={top} in the frame; clicks measured from there")
                for spec in args.click_in_window:
                    cx, dy = (int(v) for v in spec.split(","))
                    qmp_click(f, round(fw * cx / 1280), top + round(fh * dy / 800), fw, fh)
                    time.sleep(1.5)
            # Keys AFTER the clicks, because the pair is how a person opens a
            # thing: one click puts the selection on it and Enter opens it. A
            # double-click cannot be expressed here - the loop above sleeps 1.5s
            # between clicks, which is well past any double-click interval - so
            # this is the way to reach an opener from the file manager.
            for name in args.key or []:
                qmp_key(f, name)
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
        if probe_shipped_so_far(serial) and linger < PROBE_ROUND_GAP:
            print(
                f"the image ships the knowledge probe, which answers at ~{PROBE_ROUND_GAP}s; "
                f"lingering that long so there is a verdict to read"
            )
            linger = PROBE_ROUND_GAP
        if linger:
            print(f"lingering {linger}s so the journal has an after")
            time.sleep(linger)

        # Shut the guest DOWN rather than pulling its plug, so journald gets to
        # write out the rest of the boot.
        #
        # `quit` kills QEMU where it stands, and the cost of that is only visible
        # if you go looking in the overlay afterwards: the persistent journal ends
        # around 3s, well before even the serial console gives up, because
        # everything journald had not yet written back dies with the process. Every
        # boot this harness has ever run left its own record truncated that way.
        #
        # An ACPI powerdown makes systemd stop its units, which flushes the journal,
        # and then QEMU exits by itself. Bounded, because a guest that hangs on
        # shutdown must not hang the harness: if it has not gone by the deadline we
        # pull the plug after all and are no worse off than before. `-no-reboot`
        # means a guest that reboots instead of halting also just exits.
        shutdown_deadline = time.monotonic() + args.shutdown_wait
        # Whether this run ended by pulling the plug. Kept as its own fact rather
        # than inferred later from a short journal or a thin store, because those
        # are the SYMPTOMS and each has an innocent explanation of its own: a
        # component that never ran writes nothing, and so does a component that was
        # still writing when the power went. The checks below are entitled to know
        # which of those they are looking at.
        plug_pulled = False
        try:
            qmp(f, "system_powerdown")
            while time.monotonic() < shutdown_deadline:
                if proc.poll() is not None:
                    break
                time.sleep(0.25)
        except (EOFError, OSError, ValueError, QmpError):
            pass                            # already gone, or the socket died with it
        if proc.poll() is None:
            plug_pulled = True
            print(f"guest did not halt within {args.shutdown_wait}s; pulling the plug "
                  "(its journal will be short)")
            try:
                qmp(f, "quit")
            except (EOFError, OSError, ValueError, QmpError):
                pass
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
        #
        # Two explanations have since been RULED OUT by measurement, which narrows
        # what an early horizon can mean but does not yet lift it.
        #
        #   journald's rate limit   the obvious suspect, and wrong. That boot logged
        #                           1479 lines in seven seconds against a limit of
        #                           10000 per 30s, so it was never reached; turning
        #                           it off in the verify image left the horizon at
        #                           7.1s, unchanged.
        #   serial bandwidth        also wrong. 203K of log in a run with room for
        #                           1.3M at 115200, and QEMU's UART does not pace to
        #                           the configured baud anyway.
        #
        # What is left is that the console stops carrying userspace output while the
        # guest keeps running: the bar appears at 9s and contributes nothing. The
        # guest's own journal does not rescue it either - read off the overlay it
        # ends around 3s, because a passing run kills the VM instead of shutting it
        # down and journald never flushes the rest.
        #
        # That last one is the fixable one, and it is the next piece: an ACPI
        # powerdown before teardown, then read /var/log/journal out of the overlay.
        # `--keep` exists to make that read possible on a run that passed.
        try:
            with open(os.path.abspath(args.serial_out), "r", errors="replace") as fh:
                stamps = re.findall(r"^\[\s*(\d+\.\d+)\]", fh.read(), re.M)
        except OSError:
            stamps = []
        if stamps:
            last = float(stamps[-1])
            # Against how long the guest ACTUALLY ran, not against --wait.
            #
            # --wait is a ceiling the bar-polling modes never reach: they return the
            # moment the bar appears, so a `--wait 120` run is over at about eleven
            # seconds. Measured against 120 that looks like a log truncated at 9%,
            # and it is not - it is a complete log of a short run. I spent two ticks
            # on 12 Aug chasing a truncation that this line invented, first blaming
            # journald's rate limit and then the serial bandwidth, before a long
            # --linger run printed "103.7s of a ~60s run" and gave the game away.
            ran = time.monotonic() - run_start
            print(f"serial horizon: last entry at {last:.1f}s of a {ran:.0f}s run"
                  + ("  (the guest kept running past this; absence after it proves "
                     "nothing)" if last < ran * 0.6 else ""))
        else:
            print("serial horizon: no timestamped entries; the log says nothing about timing")

    # The guest's own journal, read out of the overlay now that the guest halts
    # cleanly enough to have flushed one.
    #
    # This is the only complete record a run produces. The serial console stops
    # carrying userspace output around 7-9s while the guest keeps going, so
    # everything a boot could say about steady state - the promotion pass, the
    # project watcher firing, the probe's second round - lands where nothing was
    # reading. Extracting it costs one guestfish call against a file that is about
    # to be deleted anyway.
    # An image that SHIPS the probe gets its probe asserted, asked or not.
    #
    # `--require-probe` was opt-in and nothing passed it - not CI, not the boot
    # recipe, only the README. So the refusal existed, had a control, and was armed
    # on no run: three boots reported OK past a probe nobody read. An assertion
    # nobody arms is the same defect as an assertion that cannot fail, one level up.
    #
    # The trigger is systemd's own console line for the unit, which lands around
    # 4.5s - well before the serial stops carrying userspace output - so it is
    # readable for free and does not depend on the probe having said anything. A
    # release image has no such unit and is not held to a probe it does not ship;
    # a verify image whose probe unit started and then went quiet now FAILS, which
    # is the case the flag would silently have passed.
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    from probe_verdict import probe_is_shipped

    probe_shipped = False
    try:
        with open(serial, "r", errors="replace") as fh:
            probe_shipped = probe_is_shipped(fh.read())
    except OSError:
        probe_shipped = False
    require_probe = args.require_probe or probe_shipped

    journal_text = None
    if args.journal_out or require_probe:
        jdir = os.path.join(tmp, "journal")
        os.makedirs(jdir, exist_ok=True)
        # BOTH journals, and the user one is not optional: every desktop component
        # is a user service, so a system-only capture is missing the shell, the
        # compositor, powerd, the undo signer - most of what a boot is. On 12 Aug
        # I read `0 shell lines` off a system-only capture and nearly concluded a
        # logging change had silenced the shell; the serial log had 97 of them.
        # journalctl merges multiple --file by timestamp, so the two interleave.
        script = (
            "run\nmount-ro /dev/sda2 /\n"
            f"glob copy-out /var/log/journal/*/system.journal {jdir}/\n"
            f"glob copy-out /var/log/journal/*/user-*.journal {jdir}/\n"
        )
        r = subprocess.run(["guestfish", "--ro", "-a", overlay],
                           input=script, capture_output=True, text=True)
        jfiles = sorted(
            os.path.join(jdir, f) for f in os.listdir(jdir) if f.endswith(".journal")
        ) if os.path.isdir(jdir) else []
        if r.returncode == 0 and jfiles:
            # Not `args`: that is the argparse namespace, and shadowing it here
            # made `args.journal_out` an AttributeError three lines later.
            jargs = ["journalctl"]
            for f in jfiles:
                jargs += ["--file", f]
            rendered = subprocess.run(
                jargs + ["-o", "short-iso", "--no-pager"], capture_output=True, text=True)
            if rendered.returncode == 0:
                journal_text = rendered.stdout
        if journal_text is None:
            # Say which tool was missing rather than "could not read the journal":
            # libguestfs and systemd are separate things to install, and a check
            # that cannot name its own dependency wastes the reader's next ten
            # minutes.
            print("could not read the guest journal (needs guestfish + journalctl)")
        elif args.journal_out:
            with open(os.path.abspath(args.journal_out), "w") as fh:
                fh.write(journal_text)
            print(f"journal: {os.path.abspath(args.journal_out)} "
                  f"({journal_text.count(chr(10))} lines)")

    if args.inspect:
        # The same principle as the probe below, one size down: a window saying
        # "Saved to Pictures." is the guest's account of itself, and the file
        # either exists or it does not. Read AFTER the run, out of the overlay
        # that boot wrote into - checking from a second boot would ask a machine
        # that never took the screenshot, which is a fresh overlay every time.
        # ONE guestfish per path, not one script for all of them. A path that is
        # not there makes guestfish exit non-zero, and a single script would let
        # one missing directory discard every other listing - the same trap the
        # journal reader below documents about a glob that matches nothing. A
        # missing directory is also an ANSWER here ("the app never made it"), so
        # it has to be reported rather than lost.
        for path in args.inspect:
            r = subprocess.run(
                ["guestfish", "--ro", "-a", overlay],
                input=f"run\nmount-ro /dev/sda2 /\nll {path}\n",
                capture_output=True, text=True)
            if r.returncode != 0:
                first = (r.stderr.strip().splitlines() or ["no such directory"])[-1]
                print(f"inspect {path}: NOT READ - {first}")
                continue
            print(f"inspect {path}:")
            for line in r.stdout.strip().splitlines():
                print(f"  {line}")

    if require_probe:
        # Before grading anything the guest SAID, read what the guest WROTE.
        #
        # Every assertion below this point is a self-report: the probe queries the
        # graph, prints its findings, and `probe_verdict` grades the printed lines.
        # That chain agrees with a probe that asks the wrong question or prints a
        # number it did not measure, because there is nothing in it but the probe's
        # own account of itself. So the event store comes out of the image and the
        # question gets asked again in SQL, on this side, where the guest has no
        # vote.
        #
        # Its own guestfish call rather than the journal's: a glob that matches
        # nothing makes guestfish exit non-zero, and folding this into the journal
        # script would let a moved store discard the journal too.
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        from ingest_verdict import ingest_verdict

        sdir = os.path.join(tmp, "store")
        store = None
        # A glob that matched nothing and an appliance that would not start both
        # exit non-zero here, so the reason is kept: without it the failure below
        # says "the guest wrote no event store", which is a claim about the guest
        # drawn from a fact about this machine. The appliance did fail to start
        # here tonight, under load.
        copy_problems = []
        for i, guest_path in enumerate(EVENT_STORE_PATHS):
            dest = os.path.join(sdir, str(i))
            os.makedirs(dest, exist_ok=True)
            script = f"run\nmount-ro /dev/sda2 /\nglob copy-out {guest_path} {dest}/\n"
            r = subprocess.run(["guestfish", "--ro", "-a", overlay],
                               input=script, capture_output=True, text=True)
            candidate = os.path.join(dest, "events.db")
            if r.returncode == 0 and os.path.exists(candidate):
                store = candidate
                break
            err = (r.stderr or "").strip()
            if r.returncode != 0 and "is not a file or directory" not in err:
                copy_problems.append(
                    f"{guest_path}: {err.splitlines()[-1] if err else 'guestfish failed'}"
                )

        if store is None:
            searched = ", ".join(EVENT_STORE_PATHS)
            if copy_problems:
                print(
                    "VERIFY FAIL: the event store could not be read out of the image, so "
                    "this run says nothing about ingestion: " + "; ".join(copy_problems)
                )
            else:
                print(f"VERIFY FAIL: the guest wrote no event store. Looked in {searched}")
            return 1
        ok, message = ingest_verdict(store)
        if not ok:
            # Still a failure, and still refused - but not necessarily the failure
            # the message names. A store read out of a guest that was killed
            # mid-write is missing whatever SQLite had not committed, so "the boot
            # emitted it and it never arrived" and "the boot was cut short before it
            # could arrive" produce the same empty answer. Reporting only the first
            # would send the reader after an ingestion bug that may not exist.
            if plug_pulled:
                message += (
                    ". Note the guest did not halt cleanly, so this store may be "
                    "missing writes SQLite had not committed when the plug was "
                    "pulled: fix the shutdown (or raise --shutdown-wait) before "
                    "reading this as an ingestion fault"
                )
            # The sensor's line before the return, because the two answers are
            # about different producers and the useful failure names both. A boot
            # where the desktop's own event never arrived AND the sensor forwarded
            # nothing is a different fault from one where only the desktop is
            # quiet, and returning here would have reported them identically.
            try:
                from kernel_verdict import kernel_verdict as _kv

                print(f"kernel sensor: {_kv(store)[1]}")
            except Exception as e:  # never let a diagnostic mask the real failure
                print(f"kernel sensor: not measured ({e})")
            print(f"VERIFY FAIL: {message}")
            return 1
        print(f"event store: {message}")

        # Same store, one question further out, and the one nothing was asking
        # until 18 August: the two verdicts above are both satisfied by events the
        # DESKTOP emits, so an image whose kernel sensor never loaded answers them
        # perfectly and has no machine-wide file events in it at all. The eBPF
        # program is loaded at runtime and the verifier CAN refuse it - it did, on
        # the first image that carried the sensor - and that refusal is visible
        # from nowhere except the sensor's own journal, while the unit retries and
        # the desktop comes up looking well.
        #
        # REPORTED, not required, on the --require-graph precedent: a boot can be
        # sound and still be too short for the sensor to have observed anything
        # worth writing, so gating it today would fail runs for the wrong reason.
        # `--require-kernel` is the switch for a run that means to hold it.
        from kernel_verdict import kernel_verdict

        kok, kmessage = kernel_verdict(store)
        print(f"kernel sensor: {kmessage}")
        if args.require_kernel and not kok:
            print("VERIFY FAIL: the event store holds nothing from the eBPF sensor")
            return 1

        # One hop further, and the same discipline: the graph store comes out of
        # the image too, and the host opens it with our own engine and asks. The
        # event store answers "the event arrived"; this answers "it became a File
        # node and the agent's write really linked it", which until now was only
        # ever the agent's own account of itself relayed through the journal.
        #
        # REPORTED, not required. The graph leg depends on promotion having run
        # and on the agent having a project to link to, so a boot can be sound
        # and still not have it; making it a gate today would fail runs for the
        # wrong reason. It is printed on every run so the answer is visible, and
        # `--require-graph` is the switch for a run that means to hold it.
        from graph_verdict import copy_out as copy_graph_out, run as graph_run

        gdir = os.path.join(tmp, "graph")
        os.makedirs(gdir, exist_ok=True)
        gstore, gunreadable = copy_graph_out(overlay, gdir)
        if gunreadable is not None:
            # The tooling failed, which is not the same as the guest having no
            # store, and must not be reported as one.
            gcode, gmessage = 2, f"GRAPH UNREADABLE: {gunreadable}"
        elif gstore is None:
            gcode, gmessage = 2, "GRAPH UNREADABLE: the guest wrote no graph store"
        else:
            repo_root = os.path.abspath(
                os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
            )
            gcode, gmessage = graph_run(gstore, repo_root)
        for line in gmessage.splitlines():
            print(f"graph store: {line}")
        if args.require_graph and gcode != 0:
            print("VERIFY FAIL: the graph store does not confirm this boot's write")
            return 1

        # The verdict itself lives in `probe_verdict.py` so it can be shown
        # failing: inline, the only way to exercise it was to boot an image whose
        # graph does not ingest, and there is no such image. Its control plants
        # each refusal against a synthetic journal.
        if journal_text is None:
            why = "--require-probe" if args.require_probe else "this image ships the probe"
            print(f"VERIFY FAIL: {why}, but the guest journal could not be read")
            return 1
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        from probe_verdict import probe_verdict

        ok, message = probe_verdict(journal_text)
        if not ok:
            # Same caveat as the store above: journald flushes on a clean stop, so a
            # journal from a killed guest ends early and a probe that ran fine can
            # look like a probe that never reported. The "no tally" message blames
            # --linger by name, which is the wrong place to look if the run was
            # killed before journald wrote anything back.
            if plug_pulled:
                message += (
                    ". Note the guest did not halt cleanly, so this journal is "
                    "truncated at whatever journald had flushed: check the shutdown "
                    "before reading this as a probe fault"
                )
            print(f"VERIFY FAIL: {message}")
            return 1
        print(f"knowledge probe: {message}")

    if not (os.path.exists(out) and os.path.getsize(out) > 0):
        sys.exit("no screenshot captured")
    rendered, summary = inspect(out)
    text = ocr(out)               # whole frame (psm 6) - mainly the console-text guard
    # THE FRAME A PERSON WOULD BE LOOKING AT. The dogfood consent dialog comes up
    # on every boot of this image and sits in the middle of the screen, so `out`
    # is a picture of that dialog rather than of whatever `--app` opened - three
    # app verifications in a row OCR'd a consent prompt and reported the app as
    # not having drawn. `--approve-consent` already captures an after-shot with
    # the dialog dismissed; this is what makes the app checks read it. The
    # console-text guard above deliberately keeps using the first frame: it is a
    # statement about the boot, not about what is on top of it.
    app_text = text
    if uncovered_frame and os.path.exists(uncovered_frame) and os.path.getsize(uncovered_frame) > 0:
        app_text = ocr(uncovered_frame)
        print(f"app frame: reading {os.path.basename(uncovered_frame)} (the consent dialog was dismissed)")
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
    if text is None:
        print("OCR: tesseract is not installed here, so nothing in this frame was read. "
              "The console-text guard and any --require-app-text check cannot run.")
    lower = (text or "").lower()
    console_markers = ("login:", "systemd", "audit:", "debian gnu/linux", "kernel")
    if text is not None and any(m in lower for m in console_markers):
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
            if app_text is None:
                # Not "the app did not show it": nothing looked. Saying the first
                # would send somebody hunting a rendering bug that is not there.
                print(f"VERIFY FAIL: --require-app-text needs tesseract and this host has none, "
                      f"so '{args.require_app_text}' was never looked for")
                print(f"  the frame is at {uncovered_frame or out} if you want to look yourself")
                return 1
            if want not in app_text.lower():
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
        # DOGFOOD FAIL <reason>. The stages are reported here and JUDGED in
        # `ai_verdict`, which is where the gate had to move: gating on the
        # terminal OK alone passed a boot whose AI answered nothing, because the
        # probe prints that line whatever happened. Split out so it can be shown
        # failing - `dev/vm/test_ai_verdict.py` plants that exact boot.
        from ai_verdict import ai_verdict
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
        ai_ok, ai_message = ai_verdict(journal)
        if not ai_ok:
            print(f"VERIFY FAIL: {ai_message}")
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
                verdict_line = ghost_or_resolve(serial, "Escape")
                if verdict_line is None:
                    print("consent: answered, and the card on screen is the NEXT "
                          "request the queue held - the shell took the first one "
                          "down and raised the window again")
                else:
                    print(verdict_line)
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
            # The approve click lands inside the sampled card box, so the pointer
            # that is now sitting there must be excluded; see `_blank_pointer`.
            from PIL import Image as _Im
            _w, _h = _Im.open(approved).size
            verdict, why = consent_dialog_state(
                out, approved,
                pointer_at=(round(_w * 797 / 1280), round(_h * 489 / 800)))
            frac = frame_change(out, approved)
            print(f"consent resolve: click 'Allow once' -> dialog {verdict} "
                  f"({why}; {frac*100:.1f}% of the whole frame changed) "
                  f"-> {approved}")
            if verdict == "present":
                verdict_line = ghost_or_resolve(serial, "'Allow once'")
                if verdict_line is None:
                    print("consent: answered, and the card on screen is the NEXT "
                          "request the queue held - the shell took the first one "
                          "down and raised the window again")
                else:
                    print(verdict_line)
                    print(f"  serial log: {serial}")
                    return 1
            if verdict == "inconclusive":
                # WITH `--app` THIS IS THE EXPECTED ANSWER, NOT A FAULT. The run
                # asked for a window to be opened, so the desktop the card sat on
                # is repainted by design and the "is the card gone" comparison has
                # nothing stable to compare against. Failing here made every
                # app-on-the-image screenshot exit non-zero, which is how a flag
                # combination stops being used - and using it is the only way to
                # look at an app on a real machine, since the consent card sits
                # over the middle of every unapproved frame.
                #
                # Without `--app` nothing else should have repainted, so an
                # inconclusive answer there is still a real refusal to guess.
                if args.app:
                    print("consent resolve: not graded from the frame - `--app` repaints the "
                          "desktop the card sat on, so only a person can say. "
                          f"The frame is at {approved}")
                else:
                    print("VERIFY FAIL: the gate cannot tell whether 'Allow once' "
                          "dismissed the dialog - an approve that launches something "
                          "repaints the screen the card sat on. Look at the frame; a "
                          "guess either way is a wrong verdict on a real build.")
                    print(f"  frame: {approved}")
                    return 1
    # An identity disagreement is a FAILING boot, not a line in a log.
    #
    # On 13 Aug the stamped identity tier went live and two of its own invariants
    # broke in the same evening: a session-stamped id that disagreed with the
    # `/proc` route (so the shipped profile was filed under a name nothing resolved
    # to), and a legitimate stamp thrown away by the reader. Both were visible in
    # the journal, both passed the boot, and both were found by reading it on a
    # hunch. A check that only pays out when someone goes looking is not a check.
    #
    # Deliberately NOT including `identity.broker_unauthenticated`: that one is the
    # undo signer's user namespace, a known open decision rather than a regression,
    # and a gate that is red for a reason nobody may act on teaches people to ignore
    # it.
    # Read from the SERIAL, not the guest journal: the journal is only pulled when
    # something asked for it, and a check that runs on some boots is one that will
    # be absent on the boot that needed it. These lines reach the console too -
    # verified against the same run, one hit in each.
    try:
        with open(serial, "r", errors="replace") as fh:
            identity_text = fh.read()
    except OSError:
        identity_text = ""
    if identity_text:
        broken = identity_faults(identity_text)
        if broken:
            print("VERIFY FAIL: the identity chain disagreed with itself")
            for line in broken:
                print(f"  {line}")
            print("  A divergence means two resolvers name one process differently, "
                  "and the profile is filed under only one of them; a refused stamp "
                  "means the registrar and the reader disagree about what is "
                  "legitimate. Neither is cosmetic.")
            return 1
        print("identity: no divergence and no refused stamp on the console")

        # DID THE DESKTOP START AT ALL?
        #
        # Everything below reads a boot that got somewhere. A boot where the
        # session died on its first breath still walked all the way to the end
        # and said "VERIFY OK: the compositor rendered a frame" - measured here
        # on 23 August, after a preloaded greetd config pointed at a wrapper the
        # overlay had landed without its execute bit. greetd opened the PAM
        # session at 6.07s and closed it at 6.09s, no compositor and no shell
        # ever ran, and the run passed. The frame it saw was a console.
        #
        # So: if the console was readable and never names the compositor, the
        # desktop did not start, and no amount of pixels afterwards makes that a
        # pass. Keyed on the process identifier the journal prints, which is
        # there from the compositor's first line, rather than on any one message
        # it might stop logging.
        if "arlen-compositor[" not in identity_text:
            print("VERIFY FAIL: the console never names the compositor, so the "
                  "session never started")
            print("  greetd runs /usr/bin/arlen-session, which spawns it. A "
                  "session that dies immediately leaves a bootable machine with "
                  "no desktop - and every later check here reads pixels, which a "
                  "console has too.")
            return 1

        # The socket table's other half. CI checks it against the SOURCE - every
        # dialled socket has an entry, no entry is stale - and cannot check the
        # VALUES, because the server never mentions the socket's name. The run can:
        # each daemon says what it bound. Only what this boot showed is judged, so a
        # daemon the image does not start is not evidence either way.
        table_faults = socket_table_faults(identity_text)
        if table_faults:
            print("VERIFY FAIL: the socket table disagrees with what the boot did")
            for line in table_faults:
                print(f"  {line}")
            print("  The table is a fact about which binary binds which socket. A "
                  "wrong value sends the next reader to the wrong daemon, which is "
                  "the whole cost this table exists to avoid.")
            return 1
        observed = len(observed_servers(identity_text, (_socket_table() or {}).values()))
        print(f"sockets: {observed} bind(s) on this boot, all matching the table")
    else:
        # Say so. The guard above is right - a check cannot read a console it
        # never got - but skipping in silence makes an unread serial look the same
        # as a clean identity chain, which is the failure the comment above
        # describes: the check absent on the boot that needed it.
        print("identity: NOT MEASURED - the serial log was empty or unreadable, "
              "so nothing here says the identity chain or the socket table agree")

    if not require_probe:
        # The same argument as the identity line above, one level up. Every store
        # reading in this file - the event store, the graph store, and the kernel
        # sensor - sits behind `require_probe`, which self-arms only when the image
        # ships the knowledge probe. An image built without `ARLEN_VERIFY_IMAGE=1`
        # has no probe, so the run walks past all three and ends on VERIFY OK.
        #
        # That happened on the 06:47 image of 18 August and read as a clean boot of
        # a working system. It WAS a clean boot; it just says nothing about whether
        # anything reached the graph, which is most of what this image is for.
        print("stores: NOT MEASURED - this image ships no knowledge probe, so "
              "nothing here read the event store, the graph store or the kernel "
              "sensor. Build with ARLEN_VERIFY_IMAGE=1 for a run that does")

    print("VERIFY OK: " + ("the full desktop rendered (compositor + shell bar)"
                           if bar_present else "the compositor rendered a frame"))
    # Clean run: nobody needs the overlay or the vars copy. Every failing path
    # above returns before this and leaves them where the message says they are.
    if args.keep:
        print(f"workdir kept: {tmp}")
        print(f"  the guest journal is in {tmp}/overlay.qcow2 under /var/log/journal")
    else:
        shutil.rmtree(tmp, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())

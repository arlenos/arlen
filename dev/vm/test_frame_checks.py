#!/usr/bin/env python3
"""Fixtures for the boot-verify frame checks.

The consent gate is the only automated eye on a booted system, and it has been
wrong in both directions on real boots: it called a card that was plainly on
screen `absent`, and it called a frame with 84.5% of its pixels changed "still
up". Both were caught by a human looking at the frame, which is exactly what the
gate exists to avoid needing.

So the frames that produced those verdicts are reconstructed here as synthetic
fixtures, and the detector is asserted against them. They are drawn rather than
captured because no boot frame was kept from the runs in question; each one is
built to carry the property that fooled the check, not to look like a desktop.

Run: python3 dev/vm/test_frame_checks.py
"""

import os
import sys
import tempfile

from PIL import Image, ImageDraw

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from verify import consent_dialog_state  # noqa: E402

W, H = 1280, 800


def desktop():
    """A plain desktop: a vertical gradient wallpaper and a top bar. Flat enough
    that anything drawn over it stands out, but not a single colour - a real
    wallpaper has its own spread and must not read as a dialog."""
    img = Image.new("RGB", (W, H))
    d = ImageDraw.Draw(img)
    for y in range(H):
        v = 22 + y * 18 // H
        d.line([(0, y), (W, y)], fill=(v, v, v + 6))
    d.rectangle([0, 0, W, 36], fill=(38, 38, 44))
    return img


def with_card(base, opacity=1.0, amber=True):
    """The consent card: a centred panel over a dimmed backdrop, with the amber
    header strip that marks it as a request. `opacity` fades it toward the
    backdrop the way a mid-transition frame does."""
    img = base.copy()
    d = ImageDraw.Draw(img)
    # The modal dims the whole screen behind it.
    dim = Image.new("RGB", (W, H), (0, 0, 0))
    img = Image.blend(img, dim, 0.45)
    d = ImageDraw.Draw(img)
    x0, y0, x1, y1 = int(W * 0.3), int(H * 0.3), int(W * 0.7), int(H * 0.7)

    def mix(colour):
        base_px = (12, 12, 14)
        return tuple(int(b + (c - b) * opacity) for c, b in zip(colour, base_px))

    d.rectangle([x0, y0, x1, y1], fill=mix((28, 28, 32)))
    if amber:
        d.rectangle([x0, y0 + 20, x1, y0 + 70], fill=mix((214, 150, 40)))
    for i in range(6):
        y = y0 + 110 + i * 40
        d.rectangle([x0 + 30, y, x1 - 60, y + 16], fill=mix((205, 205, 210)))
    d.rectangle([x1 - 220, y1 - 70, x1 - 120, y1 - 30], fill=mix((70, 70, 78)))
    d.rectangle([x1 - 110, y1 - 70, x1 - 20, y1 - 30], fill=mix((90, 130, 200)))
    return img


def noisy_wallpaper():
    """A busy wallpaper: legitimately high-contrast desktop. The modal dims it and
    dismissing un-dims it, which changes its brightness without anything new being
    drawn - that must not read as a window appearing."""
    img = Image.new("RGB", (W, H))
    d = ImageDraw.Draw(img)
    for y in range(0, H, 7):
        for x in range(0, W, 11):
            v = (x * 3 + y * 5) % 240
            d.rectangle([x, y, x + 11, y + 7], fill=(v, (v + 60) % 240, 200 - v // 2))
    d.rectangle([0, 0, W, 36], fill=(38, 38, 44))
    return img


def launched_app(base):
    """What an approved request looks like when it actually runs something: the
    card is gone, and a window it launched is now sitting where the card was.
    This is the frame that read as 'still up'."""
    img = base.copy()
    d = ImageDraw.Draw(img)
    d.rectangle([int(W * 0.12), int(H * 0.12), int(W * 0.88), int(H * 0.9)],
                fill=(16, 16, 18))
    d.rectangle([int(W * 0.12), int(H * 0.12), int(W * 0.88), int(H * 0.16)],
                fill=(52, 52, 60))
    for i in range(14):
        y = int(H * 0.2) + i * 26
        d.rectangle([int(W * 0.14), y, int(W * 0.14) + 380 + (i * 37) % 320, y + 12],
                    fill=(180, 220, 180))
    return img


def save(img, name, tmp):
    p = os.path.join(tmp, name)
    img.save(p)
    return p


FAILURES = []


def check(name, got, want):
    verdict, why = got
    if verdict == want:
        print(f"  ok   {name}: {verdict} ({why})")
    else:
        print(f"  FAIL {name}: wanted {want}, got {verdict} ({why})")
        FAILURES.append(name)


def main():
    with tempfile.TemporaryDirectory() as tmp:
        base = desktop()
        before_card = save(with_card(base), "before.png", tmp)
        plain = save(base, "plain.png", tmp)

        print("consent_dialog_state:")

        # The ordinary pass: the card was up, the click dismissed it, the desktop
        # is back.
        check("a dismissed card reads dismissed",
              consent_dialog_state(before_card, plain), "dismissed")

        # The ordinary fail: nothing happened, the card is untouched.
        check("an untouched card reads present",
              consent_dialog_state(before_card, before_card), "present")

        # The first observed failure direction, from the amber-only era: a card
        # caught mid-fade is still plainly on screen but its amber has dropped
        # below any threshold. It must not read as dismissed.
        faded = save(with_card(base, opacity=0.35, amber=False), "faded.png", tmp)
        check("a half-faded card is not dismissed",
              consent_dialog_state(before_card, faded), "present")

        # The second observed failure direction, and the one this rewrite is for:
        # an approve that LAUNCHES something repaints the screen the card sat on,
        # so the card's region is busier than the desktop for a reason that has
        # nothing to do with the card. The old check called this "still up" and
        # would have condemned a build that worked.
        check("an approve that launches an app is not called still-up",
              consent_dialog_state(before_card, save(launched_app(base), "app.png", tmp)),
              "inconclusive")

        # A gradient wallpaper on its own has spread too; it must not read as a card.
        check("a plain desktop against itself reads dismissed",
              consent_dialog_state(plain, plain), "dismissed")

        # The rule that keeps the launched-app case from swallowing every frame is
        # a ratio, not an absolute, because a photo wallpaper is legitimately busy
        # and dismissing the modal un-dims it. That must still read as dismissed.
        photo = noisy_wallpaper()
        check("a busy wallpaper un-dimming reads dismissed",
              consent_dialog_state(save(with_card(photo), "photo-card.png", tmp),
                                   save(photo, "photo.png", tmp)),
              "dismissed")

        # A resized frame cannot be compared at all, and saying so beats guessing.
        small = base.resize((640, 400))
        check("a frame that changed size is inconclusive",
              consent_dialog_state(before_card, save(small, "small.png", tmp)),
              "inconclusive")

    if FAILURES:
        print(f"\n{len(FAILURES)} check(s) failed: {', '.join(FAILURES)}")
        return 1
    print("\nall frame checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())

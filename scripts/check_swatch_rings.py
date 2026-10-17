#!/usr/bin/env python3
"""Check a rendered, selected + hovered swatch has one ring, not two.

Capture the real UI with the pointer left on the selected swatch, preferably
at 2x DPI. Coordinates are the button's centre in logical pixels:
  python3 scripts/check_swatch_rings.py screenshot.png --scale 2 --center 350,878
Requires ImageMagick; does not launch the app or touch the reader database.
"""
import argparse
import json
import subprocess
import tempfile
from pathlib import Path


def ink_runs(samples):
    """Number of separated ink intervals along an outward radial scan."""
    return sum(ink and (i == 0 or not samples[i - 1]) for i, ink in enumerate(samples))


def check_ring(image, center, scale, expected=1):
    x, y = center
    # The dab never reaches radius 13; both the selected ring (18) and the
    # former erroneous hover/focus ring (20.5) fall inside this strip.
    left = round(x * scale)
    top = round((y - 23) * scale)
    height = round(11 * scale) + 1
    raw = subprocess.check_output([
        "magick", str(image), "-crop", f"1x{height}+{left}+{top}",
        "+repage", "-alpha", "off", "-depth", "8", "rgb:-",
    ])
    if len(raw) != height * 3:
        raise ValueError("Radial scan falls outside the screenshot")
    pixels = list(zip(raw[0::3], raw[1::3], raw[2::3]))
    background = pixels[0]
    ink = [max(abs(a - b) for a, b in zip(pixel, background)) > 12 for pixel in pixels]
    runs = ink_runs(ink)
    if runs != expected:
        raise AssertionError(f"{center}: expected {expected} ring(s), found {runs}; scan={ink}")
    return {"center": center, "ring_count": runs}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", nargs="?")
    parser.add_argument("--center", action="append", default=[], metavar="X,Y")
    parser.add_argument("--unselected-center", action="append", default=[], metavar="X,Y",
                        help="verify an unselected, formerly focused option has no stale ring")
    parser.add_argument("--scale", type=float, default=2.0)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        assert ink_runs([False, True, True, False]) == 1
        assert ink_runs([False, True, True, False, True, False]) == 2
        assert ink_runs([False, False]) == 0
        # Exercise the complete pixel scan as well: model the old additional
        # 25%-opacity hover ring, not just a second fully opaque ring.
        with tempfile.TemporaryDirectory() as directory:
            image = Path(directory) / "ring.png"
            subprocess.run(["magick", "-size", "100x100", "xc:#f8f5ea", str(image)], check=True)
            check_ring(image, (25, 25), 2, expected=0)
            base = ["magick", "-size", "100x100", "xc:#f8f5ea", "-fill", "none",
                    "-stroke", "#7d7a71", "-strokewidth", "2.8",
                    "-draw", "circle 50,50 86,50"]
            subprocess.run(base + [str(image)], check=True)
            check_ring(image, (25, 25), 2)
            subprocess.run(base + ["-stroke", "#7d7a7140", "-strokewidth", "2",
                                   "-draw", "circle 50,50 91,50", str(image)], check=True)
            try:
                check_ring(image, (25, 25), 2)
            except AssertionError as error:
                assert "found 2" in str(error), error
            else:
                raise AssertionError("The old additional hover ring was not detected")
        print("Single-ring positive and double-ring negative controls passed")
        return
    if not args.image or not (args.center or args.unselected_center) or args.scale < 1:
        parser.error("provide a screenshot, a --center/--unselected-center, and --scale >= 1")
    results = [check_ring(args.image, tuple(map(float, value.split(","))), args.scale)
               for value in args.center]
    results += [check_ring(args.image, tuple(map(float, value.split(","))), args.scale, expected=0)
                for value in args.unselected_center]
    print(json.dumps(results, ensure_ascii=False))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Where can you hide data in a file without the pixels changing at all?

WHY THIS EXISTS
---------------
Rounds 2 and 3 found that StegaShield returns byte-identical scores when data is
appended past the end marker: 120 of 120 PNG pairs and 200 of 200 JPEG pairs. The
conclusion drawn was that it decodes an image and never reads the file.

That conclusion is worth one more hour, because the recommendation depends on
how wide it is. "Append-after-end is ignored" is a bug report. "Anything outside
the pixel data is ignored" is a design note, and the fix for it is different and
larger.

So this probes the whole class. Every variant below leaves **the decoded image
bit for bit identical** and changes only where the bytes live:

    trailing   after the end marker, which is the known case and the control
    comment    a JPEG COM segment, the format's own "put a note here" marker
    app1       a JPEG APP1 segment, which is where EXIF lives
    text       a PNG tEXt chunk, which is the PNG equivalent

If a detector scores all of them identically to the original, it is not reading
the container at any offset, and a recommendation to "check for appended data"
would be too narrow to help.

WHAT MAKES THIS A FAIR TEST
---------------------------
The pixels are verified identical after each transformation rather than assumed.
A variant that accidentally changed a sample would produce a score difference
that looks like container awareness and is not, which is the same class of
mistake that voided round 3's first outguess arms.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import sys

import numpy as np
from PIL import Image

SOI = b"\xff\xd8"
EOI = b"\xff\xd9"
COM = b"\xff\xfe"
APP1 = b"\xff\xe1"

#: Big enough that a detector reading bytes would notice, small enough to fit a
#: JPEG segment's 16-bit length field.
PAYLOAD = (b"PENTIMENTO-CONTAINER-PROBE-" + bytes(range(256)) * 12)[:4096]


def jpeg_with_segment(raw: bytes, marker: bytes, body: bytes) -> bytes:
    """Insert a marker segment directly after SOI, leaving the scan untouched.

    A JPEG segment is the marker, then a two byte big-endian length that counts
    itself, then the body. Decoders skip markers they do not recognise and COM
    is explicitly "ignore me", so the image that comes out is unchanged.
    """
    if not raw.startswith(SOI):
        raise ValueError("not a JPEG")
    length = len(body) + 2
    if length > 0xFFFF:
        raise ValueError("segment body too long for a 16 bit length")
    segment = marker + length.to_bytes(2, "big") + body
    return raw[:2] + segment + raw[2:]


def png_with_text(source: pathlib.Path, dest: pathlib.Path, body: bytes) -> None:
    """A PNG tEXt chunk, which is the format's own metadata slot."""
    from PIL.PngImagePlugin import PngInfo
    meta = PngInfo()
    meta.add_text("pentimento", body.decode("latin-1"))
    with Image.open(source) as img:
        img.load()
        img.save(dest, format="PNG", pnginfo=meta)


def pixels_of(path: pathlib.Path) -> np.ndarray:
    with Image.open(path) as img:
        img.load()
        return np.asarray(img.convert("RGB"), dtype=np.uint8)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--jpegs", required=True, help="directory of clean JPEG covers")
    ap.add_argument("--pngs", required=True, help="directory of clean PNG covers")
    ap.add_argument("--out", required=True)
    ap.add_argument("--count", type=int, default=60)
    ap.add_argument("--endpoint",
                    default="http://172.24.0.2:3000/api/analyze")
    args = ap.parse_args(argv)

    sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
    from score_arms import post_image

    sys.stdout.reconfigure(line_buffering=True)
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    jpegs = sorted(pathlib.Path(args.jpegs).glob("*.jpg"))[: args.count]
    pngs = sorted(pathlib.Path(args.pngs).glob("*.png"))[: args.count]
    if not jpegs or not pngs:
        print("need both JPEG and PNG covers", file=sys.stderr)
        return 1

    results: dict[str, list[tuple[float, float]]] = {}
    mismatched_pixels = 0
    records = []

    def probe(original: pathlib.Path, variant: pathlib.Path, label: str) -> None:
        nonlocal mismatched_pixels
        if not np.array_equal(pixels_of(original), pixels_of(variant)):
            mismatched_pixels += 1
            print(f"  {label}/{original.name}: PIXELS CHANGED, excluded",
                  file=sys.stderr)
            return
        try:
            a = post_image(args.endpoint, original)["stego_probability"]
            b = post_image(args.endpoint, variant)["stego_probability"]
        except Exception as e:  # noqa: BLE001
            print(f"  {label}/{original.name}: {e}", file=sys.stderr)
            return
        results.setdefault(label, []).append((a, b))
        records.append({"variant": label, "file": original.name,
                        "original": a, "modified": b, "identical": a == b})

    for src in jpegs:
        raw = src.read_bytes()
        for label, maker in (
            ("trailing", lambda r: r + PAYLOAD),
            ("comment", lambda r: jpeg_with_segment(r, COM, PAYLOAD)),
            ("app1", lambda r: jpeg_with_segment(r, APP1, b"Exif\x00\x00" + PAYLOAD)),
        ):
            dest = out / label / src.name
            dest.parent.mkdir(parents=True, exist_ok=True)
            if not dest.is_file():
                try:
                    dest.write_bytes(maker(raw))
                except ValueError as e:
                    print(f"  {label}/{src.name}: {e}", file=sys.stderr)
                    continue
            probe(src, dest, label)

    for src in pngs:
        dest = out / "text" / src.name
        dest.parent.mkdir(parents=True, exist_ok=True)
        if not dest.is_file():
            png_with_text(src, dest, PAYLOAD)
        probe(src, dest, "text")

    (out / "probe.jsonl").write_text(
        "".join(json.dumps(r) + "\n" for r in records)
    )

    print(f"\n{'variant':<12} {'carrier':<6} {'n':>4}  identical  mean shift")
    print("-" * 52)
    for label in ("trailing", "comment", "app1", "text"):
        pairs = results.get(label)
        if not pairs:
            continue
        identical = sum(1 for a, b in pairs if a == b)
        shift = sum(b - a for a, b in pairs) / len(pairs)
        carrier = "PNG" if label == "text" else "JPEG"
        print(f"{label:<12} {carrier:<6} {len(pairs):>4}  "
              f"{identical:>4}/{len(pairs):<4}  {shift:+.6f}")
    if mismatched_pixels:
        print(f"\n{mismatched_pixels} variant(s) excluded for changing pixels")
    print(f"\nrecords: {out / 'probe.jsonl'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

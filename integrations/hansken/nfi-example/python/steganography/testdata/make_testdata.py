#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright (C) 2026 Daniel Iwugo
"""Generate the test pictures for this example.

Run from this directory:

    python make_testdata.py

The pictures are synthetic and deterministic: a fixed seed and a fixed
encoder call, so two runs produce identical bytes and anyone can check the
fixtures are what this script says they are rather than taking it on trust.

WHY GENERATED RATHER THAN PHOTOGRAPHED
----------------------------------------
A real photograph brings a licence, an author and a subject with it, none of
which an example plugin should have to carry. These are arithmetic.

WHAT EACH ONE IS FOR
--------------------
`clean.png`         a picture with nothing after its IEND chunk
`appended.png`      the same bytes, plus a readable note stuck on the end
`clean.jpg`         a picture carrying a thumbnail, with nothing appended
`not_a_picture.gif` enough of a GIF header to be offered to the plugin and
                    refused, which is the branch this example exists to show

THE THUMBNAIL IN THE JPEG IS THE POINT OF THAT FIXTURE
--------------------------------------------------------
A camera writes a small copy of the photograph into an EXIF segment near the
front of the file. That thumbnail is itself a JPEG, so it ends with the same
two bytes, FF D9, that mark the end of the outer picture.

A plugin that finds the end of a JPEG by searching for FF D9 therefore stops
at the end of the *thumbnail* and reports everything after it as appended
data. On this fixture the two markers fall at offsets 660 and 8024, so the
search stops at 662 and reports **7,364 of 8,026 bytes, 91.8% of the file**,
as appended to a picture with nothing appended to it at all. Real photographs
almost all carry a thumbnail, so this is not a corner case.

Without the thumbnail the fixture does not test anything: a synthetic JPEG has
one FF D9, at the end, and the naive search gets it right. That was true of an
earlier version of this file, and the docstring claimed otherwise.
"""

import io
import pathlib

from PIL import Image

HERE = pathlib.Path(__file__).parent
INPUT = HERE / 'input'

SIDE = 256

#: Stuck on the end of `appended.png`. Readable on purpose: the plugin reports
#: what the trailing bytes look like, and 'text' is a more useful answer to an
#: investigator than 'unrecognised data'.
APPENDED_NOTE = (
    b'This note is not part of the picture. It sits after the IEND chunk, '
    b'which is one of the oldest ways to carry a payload inside an image file. '
    b'Everything before it is a valid PNG and every viewer will ignore this.\n'
) * 16


def gradient(side: int) -> Image.Image:
    """A smooth two-axis ramp with a diagonal seam.

    Chosen because it compresses well and is visually obvious, so a person
    looking at the fixture can see it is synthetic and not evidence.
    """
    pixels = bytearray(side * side)
    for y in range(side):
        for x in range(side):
            pixels[y * side + x] = (x * 5 // 8 + y * 3 // 8 + ((x ^ y) & 0x1F)) % 256
    return Image.frombytes('L', (side, side), bytes(pixels))


def _with_thumbnail(jpeg: bytes, picture: Image.Image) -> bytes:
    """Insert an APP1 segment carrying a small JPEG copy, as a camera does.

    Built by hand rather than through Pillow's `exif` argument so the layout
    is visible here: FF E1, a two byte length covering itself, the `Exif`
    identifier, then the thumbnail bytes.
    """
    small = io.BytesIO()
    picture.resize((48, 48)).save(small, 'JPEG', quality=60, subsampling=0, optimize=False)
    payload = b'Exif\x00\x00' + small.getvalue()
    app1 = b'\xff\xe1' + (len(payload) + 2).to_bytes(2, 'big') + payload
    return jpeg[:2] + app1 + jpeg[2:]


def main() -> None:
    INPUT.mkdir(parents=True, exist_ok=True)
    picture = gradient(SIDE)

    # optimize=False and a fixed compress_level keep the encoder's choices
    # stable, which is what makes the output reproducible.
    clean_png = INPUT / 'clean_png.raw'
    picture.save(clean_png, 'PNG', optimize=False, compress_level=6)

    (INPUT / 'appended_png.raw').write_bytes(clean_png.read_bytes() + APPENDED_NOTE)

    # quality is fixed for the same reason; subsampling is stated rather than
    # left to the encoder's default, which has changed between Pillow versions.
    jpeg = io.BytesIO()
    picture.save(jpeg, 'JPEG', quality=85, subsampling=0, optimize=False)
    (INPUT / 'clean_jpg.raw').write_bytes(_with_thumbnail(jpeg.getvalue(), picture))

    # Not a picture at all: a GIF signature and nothing that parses. The plugin
    # must refuse this rather than report it clean.
    (INPUT / 'not_a_picture.raw').write_bytes(b'GIF89a' + bytes(range(256)) * 2)

    for name in ('clean_png', 'appended_png', 'clean_jpg', 'not_a_picture'):
        size = (INPUT / f'{name}.raw').stat().st_size
        print(f'{name:16} {size:>8} bytes')


if __name__ == '__main__':
    main()

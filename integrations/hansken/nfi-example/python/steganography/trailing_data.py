# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: Apache-2.0
# Copyright (C) 2026 Daniel Iwugo
"""Find data that sits after an image file's logical end.

WHY THIS IS A SEPARATE MODULE FROM THE STATISTICAL DETECTION
------------------------------------------------------------
These are two different kinds of claim and a forensic report must not blur
them.

Trailing data is *structural*: the format says where the picture ends, and
these bytes are after it. There is no threshold, no corpus and no false
positive rate, because nothing is being estimated. Either the bytes are there
or they are not.

An LSB estimate is *statistical*: a number produced by a model of what an
untouched picture looks like, compared against a threshold that was chosen to
hold a measured false positive rate on named corpora. It can be wrong on a
picture that simply looks unusual.

An examiner needs to know which of the two they are holding, so they live in
separate modules and are written to separate trace properties.

WHAT COUNTS AS THE LOGICAL END
------------------------------
PNG ends at the IEND chunk's CRC. Everything after is trailing.

JPEG ends at the EOI marker, but EOI cannot be found by searching for the
bytes FF D9. The segment structure has to be walked, honouring byte stuffing
(FF 00) and restart markers (FF D0 to FF D7), which is what `_jpeg_end` does.

The reason is worth stating precisely, because the obvious reason is wrong.
Entropy-coded scan data cannot contain FF D9, since byte stuffing encodes a
literal FF as FF 00, and 4.6 MB of deliberately incompressible noise was
checked and produced no stray occurrence at all. A naive search is safe
against the scan.

**It is not safe against an APPn segment, and that is the case that matters,
because an EXIF thumbnail is itself a JPEG and therefore ends in FF D9.** On
the fixture built by `make_thumbnail_fixture` in the tests, a 147,023 byte
picture carrying a 2,913 byte thumbnail, the first FF D9 falls at offset
2,925, so a search reports 144,098 bytes of appended data on a picture with
nothing appended to it: 98% of the file. That fixture is generated
deterministically and every one of those figures is pinned by a test, because
an earlier version of this comment quoted them from a throwaway run that was
never saved. When the fixture was rebuilt it came out 34 bytes different, so
the old numbers were not merely uncheckable, they were wrong.

**How often that bites depends on the corpus, and the honest answer is that
it depends.** It needs a JPEG that carries a thumbnail. Straight from a
camera, most do. On the 5,000 image ALASKA2 cover sample, the only real
photograph corpus to hand, the naive search produces **zero** false positives,
because those covers were processed and carry no thumbnail. So this is a
correctness argument rather than a frequency one: the segment walk costs
almost nothing and is right on both corpora, and the search is right on only
one of them.

Taking the *last* FF D9 instead fails the other way: appended data that
itself ends in FF D9, which any appended JPEG does, moves the apparent end of
the picture to the end of the payload and hides exactly what is being looked
for.
"""
from __future__ import annotations

import dataclasses

#: Refuse to walk a chunk or segment table longer than this. The real ceiling
#: for a picture is in the low hundreds.
#:
#: **This bounds the number of segments, not the number of bytes**, and an
#: earlier version of this comment claimed otherwise. A file of 9,000 maximum
#: sized APP0 segments walks 562 MB legally and stays under the count. The
#: byte bound is `MAX_SCAN_BYTES` below, and between them the caller's own
#: size ceiling is what actually protects the worker.
MAX_CHUNKS = 10_000

#: Stop walking entropy coded data after this many bytes. Byte-at-a-time
#: scanning in Python runs at roughly 16 MB/s, so a 256 MB file of FF bytes
#: costs about 16 seconds in a worker that has no timeout on this path: the
#: subprocess timeout only covers the statistical arm. Refusing is better than
#: a stall, and refusing is not the same as reporting the picture clean.
MAX_SCAN_BYTES = 64 * 1024 * 1024

#: How much of the trailing run to look at when guessing what it is. The magic
#: numbers we know are all within the first few bytes; reading more would just
#: pull attacker-controlled data into memory for no gain.
SNIFF_BYTES = 64

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"

#: Magic numbers worth naming in a report. Deliberately short: a wrong name is
#: worse than no name, so anything not listed is reported as unrecognised
#: rather than guessed at.
_MAGIC = (
    (b"PK\x03\x04", "ZIP archive"),
    (b"PK\x05\x06", "empty ZIP archive"),
    (b"Rar!\x1a\x07", "RAR archive"),
    (b"7z\xbc\xaf\x27\x1c", "7-Zip archive"),
    (b"\x1f\x8b", "gzip stream"),
    (b"BZh", "bzip2 stream"),
    (b"\xfd7zXZ\x00", "XZ stream"),
    (b"%PDF-", "PDF document"),
    (b"\x89PNG\r\n\x1a\n", "PNG image"),
    (b"\xff\xd8\xff", "JPEG image"),
    (b"MZ", "DOS or Windows executable"),
    (b"\x7fELF", "ELF executable"),
    (b"-----BEGIN", "PEM encoded block"),
)


#: How far in to look for a signature that is not at offset zero. Data hidden
#: in front of a picture sits in the first few kilobytes in every case worth
#: naming, and a bounded window keeps this from becoming a whole file search.
PREPENDED_SEARCH_BYTES = 64 * 1024


class PrependedData(Exception):
    """A picture signature was found, but not at the start of the file.

    Its own exception rather than a ValueError, because "there are bytes in
    front of this picture" is a finding and "this is not a picture" is not.
    """

    def __init__(self, message: str, offset: int):
        super().__init__(message)
        self.offset = offset


class MalformedImage(Exception):
    """The file does not parse as the format its signature claims.

    Raised rather than returned because a caller that cannot parse the picture
    has learned nothing about whether it carries hidden data, and must not be
    allowed to record that absence as a negative result.
    """


@dataclasses.dataclass(frozen=True)
class Trailing:
    """Bytes found after the logical end of the picture."""

    #: Byte offset at which the picture's own data stops.
    offset: int
    #: How many bytes follow it.
    length: int
    #: A human readable guess at what those bytes are, or None.
    looks_like: str | None

    @property
    def present(self) -> bool:
        return self.length > 0


def _png_end(data: bytes) -> int:
    """Offset just past the IEND chunk, walking the chunk table."""
    pos = len(PNG_SIGNATURE)
    for _ in range(MAX_CHUNKS):
        # Length, type, data, CRC. A truncated header means the file ends
        # inside a chunk, which is malformed rather than clean.
        if pos + 8 > len(data):
            raise MalformedImage(f"PNG chunk header truncated at offset {pos}")
        length = int.from_bytes(data[pos:pos + 4], "big")
        kind = data[pos + 4:pos + 8]
        nxt = pos + 8 + length + 4
        if nxt > len(data):
            raise MalformedImage(
                f"PNG chunk {kind!r} at offset {pos} claims {length} bytes, "
                f"which runs past the end of the file"
            )
        if kind == b"IEND":
            return nxt
        pos = nxt
    raise MalformedImage(f"PNG has more than {MAX_CHUNKS} chunks")


#: Markers that stand alone, with no length field after them.
_JPEG_STANDALONE = {0x01} | set(range(0xD0, 0xD8))


def _jpeg_end(data: bytes) -> int:
    """Offset just past the EOI marker, walking the segment structure."""
    if data[:2] != b"\xff\xd8":
        raise MalformedImage("JPEG does not start with SOI")
    pos = 2
    for _ in range(MAX_CHUNKS):
        if pos + 2 > len(data):
            raise MalformedImage("JPEG ends without an EOI marker")
        if data[pos] != 0xFF:
            raise MalformedImage(f"expected a JPEG marker at offset {pos}")
        # A marker may be padded with any number of FF bytes.
        while pos < len(data) and data[pos] == 0xFF:
            pos += 1
        if pos >= len(data):
            raise MalformedImage("JPEG ends inside a marker")
        marker = data[pos]
        pos += 1
        if marker == 0xD9:  # EOI
            return pos
        if marker in _JPEG_STANDALONE:
            continue
        if pos + 2 > len(data):
            raise MalformedImage(f"JPEG segment length truncated at offset {pos}")
        seg_len = int.from_bytes(data[pos:pos + 2], "big")
        if seg_len < 2:
            raise MalformedImage(f"JPEG segment at offset {pos} declares length {seg_len}")
        pos += seg_len
        if marker == 0xDA:  # SOS: entropy coded data follows, scan for the next marker
            pos = _skip_entropy_coded(data, pos)
    raise MalformedImage(f"JPEG has more than {MAX_CHUNKS} segments")


def _skip_entropy_coded(data: bytes, pos: int) -> int:
    """Advance past scan data to the next real marker.

    FF 00 is a stuffed byte carrying a literal FF, and FF D0 to FF D7 are
    restart markers inside the scan. Neither ends the scan. Anything else
    preceded by FF does.
    """
    n = len(data)
    started = pos
    while pos < n:
        if pos - started > MAX_SCAN_BYTES:
            raise MalformedImage(
                f"JPEG scan data exceeds the {MAX_SCAN_BYTES} byte walking limit "
                f"without reaching a marker"
            )
        if data[pos] != 0xFF:
            pos += 1
            continue
        nxt = pos + 1
        while nxt < n and data[nxt] == 0xFF:
            nxt += 1
        if nxt >= n:
            raise MalformedImage("JPEG scan data ends inside a marker")
        b = data[nxt]
        if b == 0x00 or 0xD0 <= b <= 0xD7:
            pos = nxt + 1
            continue
        return pos
    raise MalformedImage("JPEG scan data ends without a following marker")


def _sniff(chunk: bytes) -> str | None:
    for magic, name in _MAGIC:
        if chunk.startswith(magic):
            return name
    # Printable ASCII with ordinary whitespace reads as text to an examiner,
    # and saying so is more useful than saying nothing.
    if chunk and all(0x20 <= b < 0x7F or b in (0x09, 0x0A, 0x0D) for b in chunk):
        return "text"
    return None


def find_trailing(data: bytes) -> Trailing:
    """Locate any bytes after the logical end of a PNG or JPEG.

    :raises MalformedImage: the file does not parse, so no claim can be made
        about it either way.
    :raises ValueError: the file is not a format this module handles.
    """
    if data.startswith(PNG_SIGNATURE):
        end = _png_end(data)
    elif data.startswith(b"\xff\xd8"):
        end = _jpeg_end(data)
    else:
        # A signature further in means the picture has something in front of
        # it, which is a hiding place in its own right. Reporting that as "not
        # a picture" loses the finding, so it is named instead. The search is
        # bounded because it reads a fixed window, not the whole file.
        window = data[:PREPENDED_SEARCH_BYTES]
        for signature in (PNG_SIGNATURE, b"\xff\xd8\xff"):
            at = window.find(signature)
            if at > 0:
                raise PrependedData(
                    f"{at} bytes before the start of the picture", offset=at
                )
        raise ValueError("not a PNG or JPEG")

    length = len(data) - end
    return Trailing(
        offset=end,
        length=length,
        looks_like=_sniff(data[end:end + SNIFF_BYTES]) if length else None,
    )

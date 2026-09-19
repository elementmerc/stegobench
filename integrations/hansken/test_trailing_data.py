# SPDX-License-Identifier: AGPL-3.0-or-later OR Apache-2.0
# Copyright (C) 2026 Daniel Iwugo
#
# Dual licensed, deliberately and only for this file and its tests. It is
# original work that contains no Stegcore code and depends on nothing but the
# standard library, so its licence is separable from the rest of this tree.
# Offered under Apache-2.0 as well so it can be contributed to collections
# that cannot accept copyleft, which includes NFI's own plugin examples. The
# rest of this directory, and everything it touches in Stegcore, stays
# AGPL-3.0-or-later.
"""Tests for the structural detector.

The cases that matter are the ones where a cheaper implementation gets the
wrong answer, so each of those is written as a comparison against the cheaper
implementation rather than as a bare assertion. A test that only checks the
right answer does not show that the complexity is earning its place.
"""
from __future__ import annotations

import io
import pathlib

import numpy as np
import pytest
from PIL import Image

from trailing_data import (
    PNG_SIGNATURE as PNG_SIG,
    MalformedImage,
    PrependedData,
    find_trailing,
)

FIXTURES = pathlib.Path(__file__).resolve().parents[2] / "fixtures"


def naive_first_eoi(data: bytes) -> int | None:
    """What a search for the EOI bytes would conclude. Used to show it is wrong."""
    for i in range(len(data) - 1):
        if data[i] == 0xFF and data[i + 1] == 0xD9:
            return i + 2
    return None


def noise_jpeg(size: int = 400, quality: int = 90, seed: int = 3) -> bytes:
    rng = np.random.default_rng(seed)
    img = Image.fromarray(rng.integers(0, 256, (size, size, 3), dtype=np.uint8))
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=quality)
    return buf.getvalue()


def with_exif_thumbnail(data: bytes, seed: int = 3) -> tuple[bytes, int]:
    """Insert an APP1 segment carrying a JPEG thumbnail, as a camera does."""
    rng = np.random.default_rng(seed)
    thumb = Image.fromarray(rng.integers(0, 256, (64, 64, 3), dtype=np.uint8))
    buf = io.BytesIO()
    thumb.save(buf, "JPEG", quality=70)
    tb = buf.getvalue()
    payload = b"Exif\x00\x00" + tb
    app1 = b"\xff\xe1" + (len(payload) + 2).to_bytes(2, "big") + payload
    return data[:2] + app1 + data[2:], len(tb)


def make_thumbnail_fixture() -> tuple[bytes, int]:
    """The exact picture the module docstring quotes numbers from.

    Deterministic, so the 147,023 / 2,913 / 2,925 / 144,098 figures in
    `trailing_data`'s docstring can be checked rather than believed. An earlier
    version of that comment quoted a throwaway run that was never saved; when
    the picture was rebuilt it came out 34 bytes different, so those figures
    were wrong as well as uncheckable.
    """
    return with_exif_thumbnail(noise_jpeg(size=400, quality=90, seed=3), seed=3)


class TestFixtures:
    """The six fixtures stegobench ships, which are byte identical everywhere."""

    @pytest.mark.parametrize(
        "name", ["clean.png", "clean-rgb.png", "clean.jpg", "lsb-0.4bpp.png", "lsb-0.4bpp-rgb.png"]
    )
    def test_clean_fixtures_have_nothing_trailing(self, name):
        t = find_trailing((FIXTURES / name).read_bytes())
        assert t.length == 0
        assert not t.present

    def test_appended_fixture_is_found_exactly(self):
        data = (FIXTURES / "appended.png").read_bytes()
        clean = (FIXTURES / "clean.png").read_bytes()
        t = find_trailing(data)
        # The fixture is clean.png with 4096 bytes stuck on the end, so the
        # offset must land precisely on the clean file's length.
        assert t.offset == len(clean)
        assert t.length == len(data) - len(clean) == 4096
        assert t.looks_like == "text"

    def test_lsb_fixture_is_not_flagged_structurally(self):
        """An LSB payload is invisible to this detector, and must stay invisible.

        Reporting it here would be a category error: the bytes are inside the
        picture, not after it.
        """
        assert find_trailing((FIXTURES / "lsb-0.4bpp.png").read_bytes()).length == 0


class TestExifThumbnailIsTheHardCase:
    def test_the_numbers_quoted_in_the_docstring_are_the_real_ones(self):
        """Pin every figure the module docstring states."""
        data, thumb_len = make_thumbnail_fixture()
        naive = naive_first_eoi(data)
        assert len(data) == 147023
        assert thumb_len == 2913
        assert naive == 2925
        assert len(data) - naive == 144098
        assert (len(data) - naive) / len(data) > 0.98
        assert find_trailing(data).length == 0

    def test_naive_search_is_catastrophically_wrong(self):
        data, thumb_len = with_exif_thumbnail(noise_jpeg())
        naive = naive_first_eoi(data)
        walk = find_trailing(data)
        assert walk.length == 0, "the picture is clean and must be reported clean"
        assert naive is not None and naive < walk.offset
        # Not a rounding error: a search mistakes almost the whole file for
        # appended data.
        assert (len(data) - naive) / len(data) > 0.9

    def test_appended_data_after_a_thumbnail_is_still_found(self):
        data, _ = with_exif_thumbnail(noise_jpeg())
        stego = data + b"PK\x03\x04" + b"\x00" * 500
        t = find_trailing(stego)
        assert t.length == 504
        assert t.looks_like == "ZIP archive"

    def test_last_eoi_search_would_hide_an_appended_jpeg(self):
        """The other cheap implementation, failing the other way."""
        host = noise_jpeg()
        payload = noise_jpeg(size=64, seed=11)
        stego = host + payload
        t = find_trailing(stego)
        assert t.length == len(payload)
        assert t.looks_like == "JPEG image"
        # A search for the *last* EOI lands at the end of the payload and so
        # reports nothing appended at all.
        last = max(i for i in range(len(stego) - 1) if stego[i] == 0xFF and stego[i + 1] == 0xD9)
        assert last + 2 == len(stego)


class TestScanDataCannotContainEoi:
    """The documented reason the naive search survives the scan, checked rather than assumed."""

    @pytest.mark.parametrize("size,quality", [(512, 95), (1024, 98)])
    def test_incompressible_noise_produces_no_stray_eoi(self, size, quality):
        data = noise_jpeg(size=size, quality=quality, seed=7)
        hits = [i for i in range(len(data) - 1) if data[i] == 0xFF and data[i + 1] == 0xD9]
        assert len(hits) == 1
        assert hits[0] + 2 == len(data)


class TestMalformedInputIsRefusedNotGuessed:
    def test_truncated_png(self):
        png = (FIXTURES / "clean.png").read_bytes()
        with pytest.raises(MalformedImage):
            find_trailing(png[: len(png) // 2])

    def test_png_chunk_claiming_an_absurd_length(self):
        png = bytearray((FIXTURES / "clean.png").read_bytes())
        png[8:12] = (0x7FFFFFFF).to_bytes(4, "big")
        with pytest.raises(MalformedImage):
            find_trailing(bytes(png))

    def test_truncated_jpeg(self):
        jpg = (FIXTURES / "clean.jpg").read_bytes()
        with pytest.raises(MalformedImage):
            find_trailing(jpg[: len(jpg) // 2])

    def test_jpeg_segment_length_below_the_minimum(self):
        jpg = bytearray(noise_jpeg())
        # The first segment after SOI: force its declared length to 0.
        jpg[4:6] = (0).to_bytes(2, "big")
        with pytest.raises(MalformedImage):
            find_trailing(bytes(jpg))

    def test_unknown_format_is_a_different_error(self):
        """Not a MalformedImage: we were never asked about a format we parse."""
        with pytest.raises(ValueError):
            find_trailing(b"GIF89a" + b"\x00" * 100)

    def test_empty_input(self):
        with pytest.raises(ValueError):
            find_trailing(b"")


class TestSniffing:
    @pytest.mark.parametrize(
        "magic,expected",
        [
            (b"PK\x03\x04", "ZIP archive"),
            (b"Rar!\x1a\x07", "RAR archive"),
            (b"%PDF-1.7", "PDF document"),
            (b"\x7fELF", "ELF executable"),
            (b"MZ\x90\x00", "DOS or Windows executable"),
        ],
    )
    def test_known_magic_is_named(self, magic, expected):
        data = (FIXTURES / "clean.png").read_bytes() + magic + b"\x00" * 32
        assert find_trailing(data).looks_like == expected

    def test_unrecognised_binary_is_not_guessed_at(self):
        data = (FIXTURES / "clean.png").read_bytes() + bytes([0xDE, 0xAD, 0xBE, 0xEF] * 16)
        assert find_trailing(data).looks_like is None


class TestTheRemainingRefusalPaths:
    """Error branches that had no coverage.

    Every one of these is a path where the alternative to raising is reporting
    a picture as clean, so they matter more than their line count suggests.
    """

    def test_a_png_with_too_many_chunks(self):
        from trailing_data import MAX_CHUNKS

        # A minimal valid-looking chunk repeated past the ceiling.
        chunk = (0).to_bytes(4, "big") + b"tEXt" + (0).to_bytes(4, "big")
        data = PNG_SIG + chunk * (MAX_CHUNKS + 10)
        with pytest.raises(MalformedImage, match="more than"):
            find_trailing(data)

    def test_a_png_header_truncated_mid_chunk(self):
        with pytest.raises(MalformedImage, match="truncated"):
            find_trailing(PNG_SIG + b"\x00\x00")

    def test_a_jpeg_that_is_only_a_start_marker(self):
        with pytest.raises(MalformedImage):
            find_trailing(b"\xff\xd8")

    def test_a_jpeg_ending_inside_a_marker(self):
        # Two bytes of FF padding with nothing after them: the marker never
        # arrives. `\xff\xd8\xff` alone is caught earlier as "no EOI".
        with pytest.raises(MalformedImage, match="inside a marker"):
            find_trailing(b"\xff\xd8\xff\xff")

    def test_a_jpeg_whose_segment_length_is_truncated(self):
        with pytest.raises(MalformedImage, match="length truncated"):
            find_trailing(b"\xff\xd8\xff\xe0\x00")

    def test_a_byte_that_is_not_a_marker_where_one_is_required(self):
        with pytest.raises(MalformedImage, match="expected a JPEG marker"):
            find_trailing(b"\xff\xd8\x41\x41\x41\x41")

    def test_a_standalone_marker_is_walked_past(self):
        """TEM (0x01) and the restart markers carry no length field."""
        jpg = noise_jpeg()
        with_tem = jpg[:2] + b"\xff\x01" + jpg[2:]
        assert find_trailing(with_tem).length == 0

    def test_scan_data_that_never_reaches_a_marker(self):
        from trailing_data import MAX_SCAN_BYTES

        # A scan that runs off the end without a terminating marker.
        head = noise_jpeg()[:200]
        with pytest.raises(MalformedImage):
            find_trailing(head)

    def test_the_scan_walking_limit_is_enforced(self):
        """A hostile file must be refused rather than stalling a worker."""
        from trailing_data import MAX_SCAN_BYTES

        # SOS, then more scan bytes than the limit allows, none of them a marker.
        header = b"\xff\xd8\xff\xda\x00\x03\x00"
        data = header + b"\x41" * (MAX_SCAN_BYTES + 1024)
        with pytest.raises(MalformedImage, match="walking limit"):
            find_trailing(data)

    def test_a_jpeg_with_more_segments_than_the_ceiling(self):
        from trailing_data import MAX_CHUNKS

        # Zero-payload COM segments, repeated past the count.
        segment = b"\xff\xfe\x00\x02"
        data = b"\xff\xd8" + segment * (MAX_CHUNKS + 10)
        with pytest.raises(MalformedImage, match="more than"):
            find_trailing(data)


class TestPrependedData:
    def test_a_png_behind_other_data_is_named_not_dismissed(self):
        png = (FIXTURES / "clean.png").read_bytes()
        with pytest.raises(PrependedData) as caught:
            find_trailing(b"SECRET!!" + png)
        assert caught.value.offset == 8

    def test_a_jpeg_behind_other_data(self):
        with pytest.raises(PrependedData) as caught:
            find_trailing(b"\x00" * 32 + noise_jpeg())
        assert caught.value.offset == 32

    def test_a_signature_beyond_the_search_window_is_not_found(self):
        """The window is bounded so this cannot become a whole file scan."""
        from trailing_data import PREPENDED_SEARCH_BYTES

        png = (FIXTURES / "clean.png").read_bytes()
        with pytest.raises(ValueError):
            find_trailing(b"\x00" * (PREPENDED_SEARCH_BYTES + 16) + png)

    def test_a_genuinely_unknown_format_is_still_a_plain_value_error(self):
        with pytest.raises(ValueError):
            find_trailing(b"GIF89a" + b"\x00" * 200)

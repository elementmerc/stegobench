#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the arm packer, and mostly for the licence join.

Run them with the standard library alone, because the build box's virtual
environment has no pytest in it:

    python3 -m unittest discover -s generators -p 'test_*.py'

The load bearing tests here are the ones about attribution. A stego image is a
derivative of a Commons photograph and most of those photographs require a
credit line, so a shard that ships pixels with no `cover_licence` is a licence
breach rather than a missing field. The packer is allowed to fail; it is not
allowed to ship an unattributed image and report success.

The second group covers a subtler failure. A JPEG arm names its cover
indirectly, through a positional filename in a separate pool, and an earlier
version read that name as if it were a cover name, found nothing, and wrote
`cover_licence: null` into eighty thousand samples while reporting them as
missing files.
"""
from __future__ import annotations

import json
import pathlib
import tarfile
import tempfile
import unittest

import pack_arms
from pack_arms import PackError, clean_arms, cover_of, load_jpeg_cover_map


COVER_ROWS = [
    {"file": "09710.png", "licence": "CC BY-SA 4.0", "artist": "A. Photographer",
     "credit": "Wikimedia Commons", "descriptionurl": "https://example.invalid/1"},
    {"file": "05047.png", "licence": "CC0", "artist": "B. Photographer",
     "credit": "Wikimedia Commons", "descriptionurl": "https://example.invalid/2"},
]


def write_jsonl(path: pathlib.Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(json.dumps(r) + "\n" for r in rows))


class TestJpegCoverMap(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.tmp = pathlib.Path(self._dir.name)

    def test_maps_positional_name_to_the_recorded_cover(self) -> None:
        manifest = self.tmp / "manifest.jsonl"
        write_jsonl(manifest, [
            {"clean": "clean/00000.jpg", "source_png": "09710.png"},
            {"clean": "clean/00001.jpg", "source_png": "05047.png"},
        ])
        self.assertEqual(
            load_jpeg_cover_map(manifest),
            {"00000.jpg": "09710.png", "00001.jpg": "05047.png"},
        )

    def test_repeated_rows_for_one_clean_jpeg_are_fine(self) -> None:
        # Every embedder and rate writes a row naming the same clean JPEG, so
        # agreement is the normal case and must not look like a collision.
        manifest = self.tmp / "manifest.jsonl"
        write_jsonl(manifest, [{"clean": "clean/00000.jpg",
                                "source_png": "09710.png"}] * 12)
        self.assertEqual(load_jpeg_cover_map(manifest), {"00000.jpg": "09710.png"})

    def test_a_name_pointing_at_two_covers_is_refused(self) -> None:
        manifest = self.tmp / "manifest.jsonl"
        write_jsonl(manifest, [
            {"clean": "clean/00000.jpg", "source_png": "09710.png"},
            {"clean": "clean/00000.jpg", "source_png": "05047.png"},
        ])
        with self.assertRaises(PackError):
            load_jpeg_cover_map(manifest)


class TestCoverOf(unittest.TestCase):
    MAP = {"00000.jpg": "09710.png"}

    def test_spatial_row_joins_directly(self) -> None:
        self.assertEqual(cover_of({"source_png": "09710.png"}, {}),
                         ("09710.png", "direct"))

    def test_jpeg_row_joins_through_the_clean_pool(self) -> None:
        self.assertEqual(cover_of({"source_jpeg": "00000.jpg"}, self.MAP),
                         ("09710.png", "via clean JPEG"))

    def test_jpeg_row_without_a_map_is_unresolved(self) -> None:
        self.assertEqual(cover_of({"source_jpeg": "00000.jpg"}, {}),
                         (None, "unresolved"))

    def test_row_naming_nothing_is_unresolved(self) -> None:
        self.assertEqual(cover_of({}, self.MAP), (None, "unresolved"))


class TestCleanArms(unittest.TestCase):
    def test_one_row_per_distinct_clean_image(self) -> None:
        rows = [
            {"clean": "clean_grey/00000.png", "clean_sha256": "aa",
             "source_png": "09710.png", "domain": "spatial"},
            {"clean": "clean_grey/00000.png", "clean_sha256": "aa",
             "source_png": "09710.png", "domain": "spatial"},
            {"clean": "clean_jpeg/00000.jpg", "clean_sha256": "bb",
             "source_jpeg": "00000.jpg", "domain": "jpeg-dct"},
        ]
        arms = clean_arms(rows, "adaptive")
        self.assertEqual(sorted(arms), ["clean-grey", "clean-jpeg"])
        self.assertEqual(len(arms["clean-grey"]), 1)
        self.assertEqual(arms["clean-grey"][0]["file"], "clean_grey/00000.png")
        self.assertEqual(arms["clean-grey"][0]["sha256"], "aa")

    def test_a_bare_clean_directory_is_named_after_its_group(self) -> None:
        # `jpeg-tools/clean` and `adaptive/clean_jpeg` are different encodings
        # of the same picture, so they must not collapse into one arm.
        arms = clean_arms([{"clean": "clean/00000.jpg", "clean_sha256": "cc",
                            "source_png": "09710.png"}], "jpeg-tools")
        self.assertEqual(sorted(arms), ["clean-jpeg-tools"])


class TestPackArm(unittest.TestCase):
    def setUp(self) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.tmp = pathlib.Path(self._dir.name)
        self.arms = self.tmp / "arms"
        self.out = self.tmp / "out"
        self.out.mkdir(parents=True)
        self.licences = pack_arms.load_cover_licences(self._covers())

    def _covers(self) -> pathlib.Path:
        path = self.tmp / "covers.jsonl"
        write_jsonl(path, COVER_ROWS)
        return path

    def _stego(self, rel: str, payload: bytes) -> dict:
        path = self.arms / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
        import hashlib
        return {"stego": rel, "stego_sha256": hashlib.sha256(payload).hexdigest()}

    def _members(self, shard: str) -> list[str]:
        with tarfile.open(self.out / shard) as tar:
            return tar.getnames()

    def _sample(self, shard: str, member: str) -> dict:
        with tarfile.open(self.out / shard) as tar:
            return json.loads(tar.extractfile(member).read())

    def pack(self, rows: list[dict], jpeg_map: dict[str, str], name="uerd-0050"):
        return pack_arms.pack_arm(name, rows, self.arms, self.licences, jpeg_map,
                                  self.out, 500, "stego", "stego_sha256")

    def test_jpeg_arm_inherits_its_cover_licence_through_the_map(self) -> None:
        row = self._stego("uerd/0050/00000.jpg", b"jpeg-bytes")
        row["source_jpeg"] = "00000.jpg"
        index = self.pack([row], {"00000.jpg": "09710.png"})

        self.assertEqual(index["unlicensed"], [])
        self.assertEqual(index["samples"], 1)
        sample = self._sample("pentimento-core-uerd-0050-00000.tar", "000000.json")
        self.assertEqual(sample["cover_licence"]["licence"], "CC BY-SA 4.0")
        self.assertEqual(sample["cover_licence"]["artist"], "A. Photographer")
        self.assertEqual(sample["source_png"], "09710.png")
        self.assertEqual(sample["licence_join"], "via clean JPEG")

    def test_a_sample_with_no_licence_is_left_out_rather_than_shipped(self) -> None:
        row = self._stego("uerd/0050/00000.jpg", b"jpeg-bytes")
        row["source_jpeg"] = "00000.jpg"
        index = self.pack([row], {})

        self.assertEqual(index["unlicensed"], ["uerd/0050/00000.jpg"])
        self.assertEqual(index["samples"], 0)
        self.assertEqual(index["missing"], [])
        self.assertEqual(self._members("pentimento-core-uerd-0050-00000.tar"), [])

    def test_a_missing_file_is_not_counted_as_a_licence_gap(self) -> None:
        row = {"stego": "uerd/0050/00001.jpg", "source_jpeg": "00001.jpg",
               "stego_sha256": "0" * 64}
        index = self.pack([row], {"00000.jpg": "09710.png"})

        self.assertEqual(index["missing"], ["uerd/0050/00001.jpg"])
        self.assertEqual(index["unlicensed"], [])

    def test_the_member_keeps_the_real_extension(self) -> None:
        jpeg = self._stego("uerd/0050/00000.jpg", b"jpeg-bytes")
        jpeg["source_jpeg"] = "00000.jpg"
        self.pack([jpeg], {"00000.jpg": "09710.png"})
        self.assertEqual(self._members("pentimento-core-uerd-0050-00000.tar"),
                         ["000000.jpg", "000000.json"])

        png = self._stego("wow/0050/00000.png", b"png-bytes")
        png["source_png"] = "05047.png"
        pack_arms.pack_arm("wow-0050", [png], self.arms, self.licences, {},
                           self.out, 500, "stego", "stego_sha256")
        self.assertEqual(self._members("pentimento-core-wow-0050-00000.tar"),
                         ["000000.png", "000000.json"])

    def test_a_digest_mismatch_is_its_own_category(self) -> None:
        row = self._stego("wow/0050/00000.png", b"png-bytes")
        row["source_png"] = "05047.png"
        row["stego_sha256"] = "1" * 64
        index = pack_arms.pack_arm("wow-0050", [row], self.arms, self.licences, {},
                                   self.out, 500, "stego", "stego_sha256")
        self.assertEqual(index["digest_mismatches"], ["wow/0050/00000.png"])
        self.assertEqual(index["missing"], [])
        self.assertEqual(index["unlicensed"], [])

    def test_positions_are_stable_when_a_sample_is_left_out(self) -> None:
        # The position is the join key between two arms, so a refused sample
        # has to leave a hole rather than shift every later sample up one.
        first = self._stego("wow/0050/00000.png", b"one")
        first["source_png"] = "05047.png"
        second = {"stego": "wow/0050/00001.png", "source_png": "05047.png",
                  "stego_sha256": "0" * 64}
        third = self._stego("wow/0050/00002.png", b"three")
        third["source_png"] = "09710.png"

        pack_arms.pack_arm("wow-0050", [first, second, third], self.arms,
                           self.licences, {}, self.out, 500, "stego",
                           "stego_sha256")
        self.assertEqual(self._members("pentimento-core-wow-0050-00000.tar"),
                         ["000000.png", "000000.json",
                          "000002.png", "000002.json"])


if __name__ == "__main__":
    unittest.main()


class TestMispairedRefusal(TestPackArm):
    """An arm cannot be half writer-matched and half not.

    The two kinds are not comparable: on outguess the mismatched pairing moved
    a detector from 0.50 to 0.36 with no payload involved, so averaging them
    into one arm produces a number that describes neither.
    """

    def test_a_row_without_the_arms_pairing_is_left_out(self) -> None:
        good = self._stego("outguess/0500/00000.jpg", b"one")
        good.update({"source_png": "09710.png", "pairing": "writer-matched"})
        bad = self._stego("outguess/0500/00001.jpg", b"two")
        bad["source_png"] = "05047.png"

        index = pack_arms.pack_arm("outguess-0500", [good, bad], self.arms,
                                   self.licences, {}, self.out, 500,
                                   "stego", "stego_sha256")
        self.assertEqual(index["mispaired"], ["outguess/0500/00001.jpg"])
        self.assertEqual(index["samples"], 1)
        self.assertEqual(index["unlicensed"], [])
        self.assertEqual(self._members("pentimento-core-outguess-0500-00000.tar"),
                         ["000000.jpg", "000000.json"])

    def test_an_arm_with_no_matched_rows_is_left_alone(self) -> None:
        # steghide edits coefficients in place, so none of its rows carry the
        # field and none of them should be refused for lacking it.
        rows = []
        for n in range(2):
            row = self._stego(f"steghide/0500/0000{n}.jpg", f"s{n}".encode())
            row["source_png"] = "09710.png"
            rows.append(row)
        index = pack_arms.pack_arm("steghide-0500", rows, self.arms,
                                   self.licences, {}, self.out, 500,
                                   "stego", "stego_sha256")
        self.assertEqual(index["mispaired"], [])
        self.assertEqual(index["samples"], 2)


def jpeg(app0: int = 2, quant: int = 67, payload: bytes = b"\x00" * 32) -> bytes:
    """A JPEG skeleton with a controllable number of APP0 segments.

    Enough structure for the container check to walk: the markers it reads all
    carry a length, and the scan it stops at is a real marker.
    """
    out = b"\xff\xd8"
    for _ in range(app0):
        body = b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00"
        out += b"\xff\xe0" + (len(body) + 2).to_bytes(2, "big") + body
    qt = b"\x00" * (quant - 2)
    out += b"\xff\xdb" + quant.to_bytes(2, "big") + qt
    out += b"\xff\xda\x00\x08\x01\x01\x00\x00\x3f\x00" + payload
    return out


def png(extra_chunk: bytes | None = None, idat: bytes = b"\x00" * 16) -> bytes:
    """A PNG skeleton: signature, IHDR, IDAT, optionally one more, IEND."""
    def chunk(kind: bytes, body: bytes) -> bytes:
        return (len(body).to_bytes(4, "big") + kind + body
                + b"\x00\x00\x00\x00")  # CRC is not read by container_of
    out = pack_arms.PNG_MAGIC + chunk(b"IHDR", b"\x00" * 13) + chunk(b"IDAT", idat)
    if extra_chunk:
        out += chunk(extra_chunk, b"\x01\x02")
    return out + chunk(b"IEND", b"")


class TestContainerOf(unittest.TestCase):
    """The shape comparison, on its own."""

    def test_a_jpeg_payload_does_not_change_the_container(self) -> None:
        # Different entropy-coded data after the scan, same everything before.
        self.assertEqual(pack_arms.container_of(jpeg(payload=b"\x11" * 32)),
                         pack_arms.container_of(jpeg(payload=b"\x22" * 64)))

    def test_an_extra_app0_is_a_different_container(self) -> None:
        # The exact defect: one more jpeglib write pass, one more JFIF header.
        self.assertNotEqual(pack_arms.container_of(jpeg(app0=2)),
                            pack_arms.container_of(jpeg(app0=3)))

    def test_a_different_quantisation_table_is_a_different_container(self) -> None:
        # A re-encode at another quality, which is the outguess defect.
        self.assertNotEqual(pack_arms.container_of(jpeg(quant=67)),
                            pack_arms.container_of(jpeg(quant=132)))

    def test_a_png_payload_may_change_idat_length(self) -> None:
        # Embedding changes pixels, which changes the compressed size. That is
        # the payload doing its job and must not read as a container change.
        self.assertEqual(pack_arms.container_of(png(idat=b"\x00" * 16)),
                         pack_arms.container_of(png(idat=b"\x00" * 64)))

    def test_an_added_png_chunk_is_a_different_container(self) -> None:
        self.assertNotEqual(pack_arms.container_of(png()),
                            pack_arms.container_of(png(extra_chunk=b"tEXt")))

    def test_an_unknown_format_gets_no_opinion(self) -> None:
        # Not "matched", which would wave it through, and not "mismatched",
        # which would drop a whole arm the check does not understand.
        self.assertEqual(pack_arms.container_of(b"not an image at all"), ())


class TestContainerGate(TestPackArm):
    """The gate, in the packer."""

    def _pair(self, stego_bytes: bytes, clean_bytes: bytes) -> dict:
        row = self._stego("uerd/0050/00000.jpg", stego_bytes)
        row["source_jpeg"] = "00000.jpg"
        clean = self.arms / "clean_jpeg/00000.jpg"
        clean.parent.mkdir(parents=True, exist_ok=True)
        clean.write_bytes(clean_bytes)
        row["clean"] = "clean_jpeg/00000.jpg"
        return row

    def test_a_matched_pair_packs(self) -> None:
        row = self._pair(jpeg(payload=b"\x11" * 32), jpeg(payload=b"\x00" * 32))
        index = self.pack([row], {"00000.jpg": "09710.png"})
        self.assertEqual(index["container_mismatches"], [])
        self.assertEqual(index["samples"], 1)

    def test_the_app0_defect_is_refused(self) -> None:
        """The 80,000 image bug, caught at the packer.

        Eight arms shipped with the stego half one jpeglib write further than
        the clean half. Every other check passed: digests matched, licences
        joined, counts came out right. This is the one that would have failed.
        """
        row = self._pair(jpeg(app0=3), jpeg(app0=2))
        index = self.pack([row], {"00000.jpg": "09710.png"})
        self.assertEqual(index["container_mismatches"], ["uerd/0050/00000.jpg"])
        self.assertEqual(index["samples"], 0)

    def test_the_requantisation_defect_is_refused(self) -> None:
        row = self._pair(jpeg(quant=132), jpeg(quant=67))
        index = self.pack([row], {"00000.jpg": "09710.png"})
        self.assertEqual(index["container_mismatches"], ["uerd/0050/00000.jpg"])

    def test_a_row_with_no_clean_half_is_not_gated(self) -> None:
        # The clean arms themselves have no `clean` field. Gating them would
        # refuse the control group.
        row = self._stego("uerd/0050/00000.jpg", jpeg())
        row["source_jpeg"] = "00000.jpg"
        index = self.pack([row], {"00000.jpg": "09710.png"})
        self.assertEqual(index["container_mismatches"], [])
        self.assertEqual(index["samples"], 1)

    def test_a_clean_half_that_is_not_on_disk_is_not_gated(self) -> None:
        # Absence is a different failure with its own category. Reporting it
        # here would send the diagnosis the wrong way.
        row = self._stego("uerd/0050/00000.jpg", jpeg())
        row["source_jpeg"] = "00000.jpg"
        row["clean"] = "clean_jpeg/nothing-here.jpg"
        index = self.pack([row], {"00000.jpg": "09710.png"})
        self.assertEqual(index["container_mismatches"], [])
        self.assertEqual(index["samples"], 1)

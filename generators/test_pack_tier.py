#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""What the cover packer does with a row it cannot verify.

    python3 -m unittest discover -s generators -p 'test_*.py'

WHY THIS EXISTS
---------------
`pack_tier.py` opens by promising that "every image is hashed while being
packed and checked against the digest the manifest already carries", and that
"a corpus that ships a digest it never verified is worse than one that ships
none". The check was written as `if row.get("sha256") and actual != ...`, so a
row carrying no digest at all skipped the comparison and was packed anyway: the
one case the promise is about was the one case it did not cover, and the packed
tier said nothing about which of its files had been checked.
"""
from __future__ import annotations

import hashlib
import io
import json
import pathlib
import sys
import tarfile
import tempfile
import unittest
from contextlib import redirect_stdout, redirect_stderr

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import pack_tier  # noqa: E402

PNG = bytes.fromhex(
    "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4"
    "890000000a49444154789c6300010000050001"
    "0d0a2db40000000049454e44ae426082"
)


def corpus(rows_carry_digest: list[bool]) -> tempfile.TemporaryDirectory:
    """A cover directory of one PNG per flag, with digests where asked."""
    tmp = tempfile.TemporaryDirectory()
    root = pathlib.Path(tmp.name)
    covers = root / "covers"
    covers.mkdir()
    lines = []
    for order, carries in enumerate(rows_carry_digest):
        name = f"{order:05d}.png"
        payload = PNG + bytes([order])
        (covers / name).write_bytes(payload)
        row = {"file": name, "tier_order": order}
        if carries:
            row["sha256"] = hashlib.sha256(payload).hexdigest()
        lines.append(json.dumps(row))
    (covers / "manifest.jsonl").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (root / "packed").mkdir()
    return tmp


def pack(root: pathlib.Path, count: int) -> tuple[int, str]:
    out = io.StringIO()
    with redirect_stdout(out), redirect_stderr(out):
        code = pack_tier.main([
            "--covers", str(root / "covers"),
            "--out", str(root / "packed"),
            "--count", str(count),
            "--per-shard", "10",
        ])
    return code, out.getvalue()


def members(root: pathlib.Path) -> list[str]:
    names: list[str] = []
    for shard in sorted((root / "packed").glob("*.tar")):
        with tarfile.open(shard) as tar:
            names.extend(sorted(tar.getnames()))
    return names


class DigestTests(unittest.TestCase):
    def test_a_row_with_no_digest_is_refused_rather_than_packed(self):
        with corpus([True, False, True]) as name:
            root = pathlib.Path(name)
            code, output = pack(root, 3)
            packed = members(root)
        self.assertEqual(code, 1, output)
        self.assertIn("NO DIGEST", output)
        # The unverified cover is position 1, and it is not in the archive.
        self.assertNotIn("000001.png", packed)
        self.assertIn("000000.png", packed)

    def test_a_digest_that_does_not_match_is_still_refused(self):
        with corpus([True, True]) as name:
            root = pathlib.Path(name)
            manifest = root / "covers" / "manifest.jsonl"
            lines = manifest.read_text(encoding="utf-8").splitlines()
            row = json.loads(lines[1])
            row["sha256"] = "0" * 64
            lines[1] = json.dumps(row)
            manifest.write_text("\n".join(lines) + "\n", encoding="utf-8")
            code, output = pack(root, 2)
        self.assertEqual(code, 1, output)
        self.assertIn("DIGEST MISMATCH", output)

    def test_a_shard_reports_what_it_packed_rather_than_what_was_planned(self):
        # The index is what every later reader walks, so a shard that says it
        # holds three samples and holds two turns a failed run's output into
        # something indistinguishable from a finished release.
        with corpus([True, False, True]) as name:
            root = pathlib.Path(name)
            pack(root, 3)
            index = json.loads(
                next((root / "packed").glob("*-index.json")).read_text(encoding="utf-8"))
        self.assertEqual([s["samples"] for s in index["shards"]], [2])
        self.assertEqual(index["samples"], 2)

    def test_a_fully_verified_tier_packs_and_exits_zero(self):
        with corpus([True, True, True]) as name:
            root = pathlib.Path(name)
            code, output = pack(root, 3)
            packed = members(root)
        self.assertEqual(code, 0, output)
        self.assertEqual(
            packed,
            ["000000.json", "000000.png", "000001.json", "000001.png",
             "000002.json", "000002.png"],
        )


if __name__ == "__main__":
    unittest.main()

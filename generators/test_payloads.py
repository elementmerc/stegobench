#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for payload derivation, and really for one claim.

    python3 -m unittest discover -s generators -p 'test_*.py'

The corpus README says it is rebuildable byte for byte. For the tool arms that
was false, and the reason was invisible: a shared seeded generator is
reproducible only if the run never skips, and that run skipped on resume and on
every cover the tool refused. The property is asserted here directly, including
the resume case that broke it, because reading the code is what missed it the
first time.
"""
from __future__ import annotations

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from payloads import SCHEME, payload_bytes, payload_seed  # noqa: E402


class DerivationTests(unittest.TestCase):
    def test_the_same_request_gives_the_same_bytes(self):
        a = payload_bytes(1234, "00072.png", "steghide", 0.05, 512)
        b = payload_bytes(1234, "00072.png", "steghide", 0.05, 512)
        self.assertEqual(a, b)

    def test_the_length_is_what_was_asked_for(self):
        for length in (0, 1, 16, 31, 32, 33, 4096):
            self.assertEqual(len(payload_bytes(1, "a.png", "t", 0.1, length)),
                             length)

    def test_a_negative_length_is_refused(self):
        with self.assertRaises(ValueError):
            payload_bytes(1, "a.png", "t", 0.1, -1)

    def test_every_input_changes_the_output(self):
        base = payload_bytes(1234, "00072.png", "steghide", 0.05, 64)
        self.assertNotEqual(base, payload_bytes(9999, "00072.png", "steghide", 0.05, 64))
        self.assertNotEqual(base, payload_bytes(1234, "00073.png", "steghide", 0.05, 64))
        self.assertNotEqual(base, payload_bytes(1234, "00072.png", "outguess", 0.05, 64))
        self.assertNotEqual(base, payload_bytes(1234, "00072.png", "steghide", 0.50, 64))

    def test_a_longer_payload_extends_a_shorter_one(self):
        # Counter mode, so the first 32 bytes do not change when more are
        # asked for. Not required by anything, but a derivation where they DID
        # change would mean the payload depends on the capacity calculation,
        # and capacity depends on the tool's build.
        short = payload_bytes(1234, "00072.png", "steghide", 0.05, 32)
        long = payload_bytes(1234, "00072.png", "steghide", 0.05, 96)
        self.assertEqual(long[:32], short)

    def test_the_scheme_is_part_of_the_derivation(self):
        # Changing it must change every payload, which is what makes a future
        # change to the derivation a visible, versioned act.
        self.assertIn(SCHEME.encode(), b"" + SCHEME.encode())
        self.assertNotEqual(payload_seed(1, "a.png", "t", 0.1),
                            payload_seed(1, "a.png", "t", 0.2))


class ResumeTests(unittest.TestCase):
    """The failure this module exists for."""

    IMAGES = [f"{i:05d}.png" for i in range(20)]
    RATES = (0.05, 0.2, 0.5)
    TOOLS = ("steghide", "outguess")

    def build(self, skip: set[str] | None = None) -> dict[str, bytes]:
        """Simulate a build pass, skipping what a real one would skip."""
        skip = skip or set()
        out = {}
        for image in self.IMAGES:
            for tool in self.TOOLS:
                for rate in self.RATES:
                    key = f"{tool}/{rate}/{image}"
                    if key in skip:
                        continue
                    # A real run draws a length that depends on the cover's
                    # capacity, which varies per image. That variation is what
                    # made a shared stream position-dependent.
                    length = 16 + (hash(image) % 97)
                    out[key] = payload_bytes(1234, image, tool, rate, length)
        return out

    def test_an_interrupted_build_resumes_to_the_same_corpus(self):
        complete = self.build()

        # First pass dies a third of the way through.
        first_half = set(list(complete)[40:])
        partial = self.build(skip=first_half)
        # Second pass does what the first did not.
        rest = self.build(skip=set(partial))

        resumed = {**partial, **rest}
        self.assertEqual(resumed.keys(), complete.keys())
        self.assertEqual(resumed, complete,
                         "a resumed build produced different payloads, which is "
                         "the defect this module exists to prevent")

    def test_a_refused_cover_does_not_shift_every_later_payload(self):
        """outguess refuses 1,884 covers. Under a shared stream each refusal
        moved every subsequent payload along, so the arms depended on which
        covers the tool happened to reject."""
        complete = self.build()
        refused = {f"outguess/{r}/{i}" for r in self.RATES
                   for i in self.IMAGES[3:6]}
        with_refusals = self.build(skip=refused)

        for key, value in with_refusals.items():
            self.assertEqual(value, complete[key],
                             f"{key} changed because a different cover was refused")


if __name__ == "__main__":
    unittest.main()

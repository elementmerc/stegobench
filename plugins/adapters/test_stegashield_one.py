#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the StegaShield host adapter.

Nothing here touches the network. The adapter is how a detector that runs as a
service gets scored, so it is the piece an outside team meets first, and until
now nothing under `plugins/adapters/` had a test at all.

The refusal message matters as much as the behaviour here. A team pointing this
at their own instance for the first time will see it, and it is the only place
that tells them what to export.
"""

import io
import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import stegashield_one as adapter  # noqa: E402


class Answered:
    """A urlopen context manager answering with fixed bytes."""

    def __init__(self, payload: bytes) -> None:
        self.payload = payload

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def read(self, size=None):
        return self.payload if size is None else self.payload[:size]


class SafeFilenameTests(unittest.TestCase):
    def test_an_ordinary_name_is_left_alone(self):
        self.assertEqual(adapter.safe_filename("09710.png"), "09710.png")

    def test_a_quote_cannot_end_the_header_early(self):
        self.assertNotIn('"', adapter.safe_filename('a".png'))

    def test_a_line_ending_cannot_start_a_header_of_its_own(self):
        cleaned = adapter.safe_filename("a\r\nX-Injected: yes.png")
        self.assertNotIn("\r", cleaned)
        self.assertNotIn("\n", cleaned)

    def test_a_backslash_cannot_escape_the_closing_quote(self):
        self.assertNotIn("\\", adapter.safe_filename("a\\.png"))

    def test_an_over_long_name_is_bounded(self):
        self.assertLessEqual(len(adapter.safe_filename("a" * 4000)), 255)

    def test_a_name_that_is_entirely_unsafe_still_yields_something(self):
        self.assertEqual(adapter.safe_filename('"""'), "___")
        self.assertEqual(adapter.safe_filename(""), "image")


class PostImageTests(unittest.TestCase):
    def image(self, tmp, name="x.png"):
        path = pathlib.Path(tmp) / name
        path.write_bytes(b"\x89PNG\r\n\x1a\n")
        return path

    def test_a_small_answer_is_parsed(self):
        with tempfile.TemporaryDirectory() as tmp:
            payload = json.dumps({"stego_probability": 0.25}).encode()
            with mock.patch.object(adapter.urllib.request, "urlopen", return_value=Answered(payload)):
                got = adapter.post_image("https://e/api/analyze", self.image(tmp))
        self.assertEqual(got["stego_probability"], 0.25)

    def test_an_enormous_answer_is_refused_rather_than_truncated(self):
        # A truncated JSON document either fails to parse or parses into
        # something that is not what was sent, and the second is worse.
        with tempfile.TemporaryDirectory() as tmp:
            payload = b"x" * (adapter.MAX_RESPONSE_BYTES + 10)
            with mock.patch.object(adapter.urllib.request, "urlopen", return_value=Answered(payload)):
                with self.assertRaises(ValueError) as caught:
                    adapter.post_image("https://e/api/analyze", self.image(tmp))
        self.assertIn(str(adapter.MAX_RESPONSE_BYTES), str(caught.exception))

    def test_the_multipart_header_carries_the_cleaned_name(self):
        seen = {}

        def capture(req, timeout=None):
            seen["body"] = req.data
            return Answered(json.dumps({"stego_probability": 0.0}).encode())

        with tempfile.TemporaryDirectory() as tmp:
            path = self.image(tmp, 'we"ird.png')
            with mock.patch.object(adapter.urllib.request, "urlopen", capture):
                adapter.post_image("https://e/api/analyze", path)
        header = seen["body"].split(b"\r\n\r\n", 1)[0]
        self.assertIn(b'filename="we_ird.png"', header)
        self.assertEqual(header.count(b"Content-Disposition"), 1)


class MainTests(unittest.TestCase):
    def run_main(self, argv, env):
        err = io.StringIO()
        with mock.patch.dict(adapter.os.environ, env, clear=True):
            with mock.patch.object(sys, "stderr", err):
                code = adapter.main(argv)
        return code, err.getvalue()

    def test_no_endpoint_refuses_and_says_exactly_what_to_export(self):
        # This is the first thing a team pointing it at their own instance
        # sees. "not set" alone sends them to read the source.
        code, err = self.run_main(["stegashield_one.py", "x.png"], {})
        self.assertEqual(code, 3)
        self.assertIn("STEGASHIELD_ENDPOINT=http://<host>:<port>/api/analyze", err)
        self.assertIn("3000", err)

    def test_the_route_keeps_the_vendor_spelling(self):
        # The docstring once said /api/analyse. The real route is /api/analyze,
        # and a guide that sends a reader to a route which does not exist is
        # worse than one that says nothing.
        code, err = self.run_main(["stegashield_one.py", "x.png"], {})
        self.assertIn("/api/analyze", err)
        self.assertNotIn("/api/analyse", err)
        self.assertNotIn("/api/analyse", adapter.__doc__)

    def test_a_whitespace_only_endpoint_counts_as_unset(self):
        code, _ = self.run_main(
            ["stegashield_one.py", "x.png"], {"STEGASHIELD_ENDPOINT": "   "}
        )
        self.assertEqual(code, 3)

    def test_wrong_argument_count_is_a_usage_error(self):
        self.assertEqual(self.run_main(["stegashield_one.py"], {})[0], 2)
        self.assertEqual(self.run_main(["a", "b", "c"], {})[0], 2)

    def test_a_missing_image_is_reported_by_path(self):
        code, err = self.run_main(
            ["stegashield_one.py", "/no/such/image.png"],
            {"STEGASHIELD_ENDPOINT": "https://e/api/analyze"},
        )
        self.assertEqual(code, 1)
        self.assertIn("/no/such/image.png", err)

    def test_an_unreachable_service_names_the_endpoint(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "x.png"
            path.write_bytes(b"\x89PNG")
            with mock.patch.object(
                adapter,
                "post_image",
                side_effect=adapter.urllib.error.URLError("no route to host"),
            ):
                code, err = self.run_main(
                    ["stegashield_one.py", str(path)],
                    {"STEGASHIELD_ENDPOINT": "https://e/api/analyze"},
                )
        self.assertEqual(code, 4)
        self.assertIn("https://e/api/analyze", err)

    def test_a_missing_score_never_falls_back_to_zero(self):
        # Zero would read as the most confident possible "clean", which is the
        # opposite of not knowing, and it would land in a published number.
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "x.png"
            path.write_bytes(b"\x89PNG")
            with mock.patch.object(adapter, "post_image", return_value={"verdict": "clean"}):
                code, err = self.run_main(
                    ["stegashield_one.py", str(path)],
                    {"STEGASHIELD_ENDPOINT": "https://e/api/analyze"},
                )
        self.assertEqual(code, 1)
        self.assertIn("no stego_probability", err)

    def test_a_score_is_printed_at_a_fixed_precision(self):
        out = io.StringIO()
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "x.png"
            path.write_bytes(b"\x89PNG")
            with mock.patch.object(adapter, "post_image", return_value={"stego_probability": 0.5}):
                with mock.patch.dict(
                    adapter.os.environ,
                    {"STEGASHIELD_ENDPOINT": "https://e/api/analyze"},
                    clear=True,
                ):
                    with mock.patch.object(sys, "stdout", out):
                        code = adapter.main(["stegashield_one.py", str(path)])
        self.assertEqual(code, 0)
        self.assertEqual(out.getvalue().strip(), "0.5000000000")


class ShippedEntryTests(unittest.TestCase):
    def test_the_registry_entry_ships_no_address(self):
        # The check that matters lives in the Rust registry reader; this is the
        # cheap one that runs wherever Python does.
        entry = (
            pathlib.Path(__file__).resolve().parents[1]
            / "registry"
            / "detectors"
            / "stegashield.toml"
        )
        text = entry.read_text(encoding="utf-8")
        for line in text.splitlines():
            stripped = line.strip()
            if stripped.startswith("#") or not stripped:
                continue
            self.assertNotIn("172.24.", stripped, f"a private address is live in {entry}")
            self.assertNotIn("127.0.0.1", stripped)
            self.assertNotIn("localhost", stripped)


if __name__ == "__main__":
    unittest.main()

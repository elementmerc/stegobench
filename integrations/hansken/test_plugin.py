# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for `process()` itself, driven through a stand-in trace.

The SDK's own framework compares against golden results recorded WITH the
analyser present, so it covers exactly one mode and fails for anyone who runs
`tox` without a build of Stegcore. The degraded path, which the README
advertises as the interesting one, had no test at all.

These use a fake trace rather than the SDK harness so every branch is
reachable, including the ones the harness cannot construct.
"""
from __future__ import annotations

import os
import pathlib
import shutil

import pytest

import plugin as plugin_module
from plugin import MAX_PICTURE_BYTES, NS, SteganographyPlugin

FIXTURES = pathlib.Path(__file__).resolve().parents[2] / "fixtures"
MISSING_BINARY = "/nonexistent/stegcore"


def _analyser() -> str | None:
    """Where a build of the analyser is, if this machine has one.

    Resolved the same way `plugin.py` resolves it, so a machine that can run
    the plugin can run these tests, and one that cannot skips them.
    """
    declared = os.environ.get("STEGCORE_BINARY")
    if declared:
        return declared if pathlib.Path(declared).exists() else None
    return shutil.which("stegcore")


ANALYSER = _analyser()


class FakeTrace:
    """The smallest thing `process()` can work against."""

    def __init__(self, data: bytes, properties=None):
        self._data = data
        self._properties = properties or {}
        self.written: dict[str, str] = {}

    def get(self, key, default=None):
        return self._properties.get(key, default)

    def open(self):
        trace = self

        class Reader:
            def __enter__(self_inner):
                return self_inner

            def __exit__(self_inner, *exc):
                return False

            def read(self_inner, n):
                return trace._data[:n]

        return Reader()

    def update(self, key_or_updates, value=None):
        if isinstance(key_or_updates, dict):
            self.written.update({k: str(v) for k, v in key_or_updates.items()})
        else:
            self.written[key_or_updates] = str(value)


class FakeContext:
    def __init__(self, size, data_type="raw"):
        self.data_size = size
        self.data_type = data_type


def run_plugin(data: bytes, properties=None, binary=MISSING_BINARY, size=None):
    trace = FakeTrace(data, properties)
    original = plugin_module.STEGCORE_BINARY
    plugin_module.STEGCORE_BINARY = binary
    try:
        SteganographyPlugin().process(trace, FakeContext(size if size is not None else len(data)))
    finally:
        plugin_module.STEGCORE_BINARY = original
    return trace.written


class TestDegradedMode:
    """M2: the mode with no analyser, which had no coverage."""

    def test_the_structural_finding_survives_without_the_analyser(self):
        written = run_plugin((FIXTURES / "appended.png").read_bytes())
        assert written[f"{NS}.stegStructural"] == "appended data"
        assert written[f"{NS}.stegAppendedBytes"] == "4096"
        assert written[f"{NS}.stegAppendedLooksLike"] == "text"

    def test_the_statistical_property_says_why_rather_than_going_quiet(self):
        written = run_plugin((FIXTURES / "clean.png").read_bytes())
        assert written[f"{NS}.stegStatistical"].startswith("not assessed:")

    def test_a_clean_picture_is_never_silently_cleared(self):
        """Every trace must carry both properties, always."""
        for name in ("clean.png", "clean.jpg", "lsb-0.4bpp.png", "appended.png"):
            written = run_plugin((FIXTURES / name).read_bytes())
            assert f"{NS}.stegStructural" in written, name
            assert f"{NS}.stegStatistical" in written, name

    def test_no_calibration_claim_is_made_when_nothing_was_measured(self):
        written = run_plugin((FIXTURES / "clean.png").read_bytes())
        assert f"{NS}.stegCalibration" not in written


class TestBoundaryInputs:
    def test_a_numeric_filename_does_not_crash_the_trace(self):
        """`name.lower()` on an int took the whole of process() down.

        The structural finding had already been written at that point, so the
        trace ended up with half an answer and no error marker.
        """
        written = run_plugin((FIXTURES / "clean.png").read_bytes(), properties={"file.name": 12345})
        assert f"{NS}.stegStructural" in written
        assert f"{NS}.stegStatistical" in written

    def test_a_trace_with_no_name_at_all(self):
        written = run_plugin((FIXTURES / "clean.png").read_bytes(), properties={})
        assert written[f"{NS}.stegStructural"] == "none"

    def test_an_oversize_picture_records_both_properties(self):
        """A skip marker alone made the trace invisible to a gap query."""
        written = run_plugin(b"\x89PNG\r\n\x1a\n", size=MAX_PICTURE_BYTES + 1)
        assert "not assessed" in written[f"{NS}.stegStructural"]
        assert "not assessed" in written[f"{NS}.stegStatistical"]

    def test_a_file_that_is_not_a_picture(self):
        written = run_plugin(b"GIF89a" + b"\x00" * 64)
        assert written[f"{NS}.stegStructural"] == "not assessed: not a PNG or JPEG"

    def test_a_truncated_picture_is_not_reported_clean(self):
        png = (FIXTURES / "clean.png").read_bytes()
        written = run_plugin(png[: len(png) // 2])
        assert written[f"{NS}.stegStructural"].startswith("not assessed:")

    def test_an_empty_stream(self):
        written = run_plugin(b"")
        assert written[f"{NS}.stegStructural"].startswith("not assessed:")

    def test_data_in_front_of_the_picture_is_named(self):
        """Prepended data is a hiding place, not an unrecognised file."""
        png = (FIXTURES / "clean.png").read_bytes()
        written = run_plugin(b"HIDDEN!!" + png)
        assert written[f"{NS}.stegStructural"] == "prepended data"
        assert written[f"{NS}.stegPrependedBytes"] == "8"


@pytest.mark.skipif(
    ANALYSER is None,
    reason="no build of the analyser: set STEGCORE_BINARY or put stegcore on PATH",
)
class TestWithTheAnalyser:
    BINARY = ANALYSER

    def test_a_jpeg_is_not_assessed_rather_than_cleared(self):
        written = run_plugin(
            (FIXTURES / "clean.jpg").read_bytes(),
            properties={"file.name": "clean.jpg"},
            binary=self.BINARY,
        )
        assert written[f"{NS}.stegStatistical"].startswith("not assessed:")
        assert f"{NS}.stegCalibration" not in written

    def test_an_lsb_png_fires_and_carries_its_calibration(self):
        written = run_plugin(
            (FIXTURES / "lsb-0.4bpp.png").read_bytes(),
            properties={"file.name": "lsb-0.4bpp.png"},
            binary=self.BINARY,
        )
        assert written[f"{NS}.stegStatistical"] == "above threshold"
        assert "false positive rate" in written[f"{NS}.stegCalibration"]

    def test_a_clean_png_stays_below_threshold(self):
        written = run_plugin(
            (FIXTURES / "clean.png").read_bytes(),
            properties={"file.name": "clean.png"},
            binary=self.BINARY,
        )
        assert written[f"{NS}.stegStatistical"] == "below threshold"

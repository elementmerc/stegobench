# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the Stegcore translator, against captured real output.

The documents in `testdata/stegcore-output/` came out of the release binary on
the six fixtures stegobench ships. Capturing them means the mapping is tested
against what the tool actually prints rather than against what this module
hopes it prints, and it means these tests run with no binary present.
"""
from __future__ import annotations

import json
import os
import pathlib

import pytest

from stegcore_adapter import (
    CALIBRATION,
    NON_DECIDING,
    SPATIAL_FORMATS,
    Analysis,
    StegcoreUnavailable,
    parse,
    preflight,
    run,
)

CAPTURED = pathlib.Path(__file__).parent / "testdata" / "stegcore-output"


def load(name: str) -> Analysis:
    return parse((CAPTURED / name).read_bytes())


class TestSpatialPictures:
    def test_clean_png_fires_nothing(self):
        a = load("clean-png.json")
        assert a.assessed
        assert a.firing == ()
        assert a.fingerprint is None

    def test_lsb_png_fires_all_three_calibrated_detectors(self):
        a = load("lsb-0.4bpp-png.json")
        assert a.assessed
        assert {d.name for d in a.firing} == set(CALIBRATION["thresholds"])

    def test_appended_png_carries_a_fingerprint_with_its_tier(self):
        a = load("appended-png.json")
        assert a.fingerprint is not None
        assert a.fingerprint_tier == "heuristic"

    def test_the_margin_is_real_and_not_a_hair(self):
        """A threshold that clean and stego both sit within is not a threshold."""
        clean = {d.name: d.score for d in load("clean-png.json").detectors}
        stego = {d.name: d.score for d in load("lsb-0.4bpp-png.json").detectors}
        for name, threshold in CALIBRATION["thresholds"].items():
            assert clean[name] < threshold < stego[name], name


class TestTheJpegRefusal:
    """The behaviour this plugin exists for."""

    def test_jpeg_is_not_assessed_rather_than_reported_clean(self):
        a = load("clean-jpg.json")
        assert not a.assessed
        assert a.detectors is None

    def test_stegcore_itself_would_have_said_clean(self):
        """Documenting the divergence, so nobody 'fixes' it later.

        The binary's own verdict on this file is `clean`. That verdict is
        correct in the sense that the file carries nothing, and useless in the
        sense that the same verdict would come back from a JPEG stuffed full
        of payload, because the ensemble is blind in the DCT domain. The
        plugin therefore declines to pass the verdict through.
        """
        raw = json.loads((CAPTURED / "clean-jpg.json").read_bytes())
        assert raw["data"][0]["verdict"] == "clean"
        assert not load("clean-jpg.json").assessed

    def test_the_scores_are_still_reported_for_completeness(self):
        a = load("clean-jpg.json")
        assert "Sample Pair Analysis" in a.reported


class TestNonDecidingDetectors:
    def test_chi_squared_and_entropy_are_excluded_from_the_verdict(self):
        a = load("lsb-0.4bpp-png.json")
        assert {d.name for d in a.detectors}.isdisjoint(NON_DECIDING)

    def test_they_are_still_reported(self):
        a = load("lsb-0.4bpp-png.json")
        for name in NON_DECIDING:
            assert name in a.reported

    def test_the_evidence_for_excluding_them_still_holds(self):
        """If these ever start carrying information, this test should fail.

        They are excluded because they return the same value on a clean
        picture and on one carrying 0.4 bpp. That is a measurement, and a
        measurement can change when the detectors change, so it is pinned
        rather than trusted.
        """
        clean = load("clean-png.json").reported
        stego = load("lsb-0.4bpp-png.json").reported
        for name in NON_DECIDING:
            assert abs(clean[name] - stego[name]) < 0.01, (
                f"{name} now separates clean from stego by "
                f"{abs(clean[name] - stego[name]):.4f}; revisit its exclusion"
            )


class TestMalformedOutputIsNeverReadAsClean:
    def test_not_json(self):
        with pytest.raises(StegcoreUnavailable):
            parse(b"this is not json")

    def test_ok_false(self):
        with pytest.raises(StegcoreUnavailable):
            parse(json.dumps({"ok": False, "data": []}).encode())

    def test_empty_data(self):
        with pytest.raises(StegcoreUnavailable):
            parse(json.dumps({"ok": True, "data": []}).encode())

    def test_more_than_one_entry(self):
        entry = json.loads((CAPTURED / "clean-png.json").read_bytes())["data"][0]
        with pytest.raises(StegcoreUnavailable):
            parse(json.dumps({"ok": True, "data": [entry, entry]}).encode())

    def test_an_uncalibrated_detector_stops_the_analysis(self):
        """A new detector must not be quietly ignored.

        If Stegcore gains a detector and this plugin keeps shipping, the
        honest failure is to refuse, because the alternative is an examiner
        reading a verdict that silently excluded evidence.
        """
        doc = json.loads((CAPTURED / "clean-png.json").read_bytes())
        doc["data"][0]["tests"].append(
            {"name": "Rich Model", "score": 0.9, "confidence": "high", "detail": ""}
        )
        with pytest.raises(StegcoreUnavailable, match="no calibrated threshold"):
            parse(json.dumps(doc).encode())


#: The record these constants are transcribed from. It lives outside this
#: repository, so a checkout without it skips rather than fails. Point
#: STEGCORE_CALIBRATION at your own copy to run this check; hard coding one
#: person's home directory made it unrunnable for everybody else.
CALIBRATION_SOURCE = pathlib.Path(
    os.environ.get("STEGCORE_CALIBRATION", "calibration/recal-final.json")
)


class TestCalibrationRecord:
    def test_the_thresholds_match_the_calibration_file(self):
        """Read the file this claims to come from, rather than restating it.

        The earlier version of this test asserted the literals against the
        literals in `stegcore_adapter`, with a docstring naming a source file
        it never opened. It could not detect drift from the thing it existed
        to guard.

        Source: `recal-final.json`, key `combined_4pct`, recalibrated
        2026-06-14.
        """
        if not CALIBRATION_SOURCE.exists():
            pytest.skip(f"{CALIBRATION_SOURCE} is not on this machine")
        recorded = json.loads(CALIBRATION_SOURCE.read_text(encoding="utf-8"))["combined_4pct"]
        assert CALIBRATION["thresholds"] == {
            "Sample Pair Analysis": recorded["spa"],
            "RS Analysis": recorded["rs"],
            "Weighted Stego": recorded["ws"],
        }

    def test_the_worst_corpus_fpr_is_the_one_quoted(self):
        """The headline rate must not be better than the worst sub-distribution.

        Quoting 0.0% because Cassavia scored 0.0% would be the exact error the
        2026-06-14 recalibration was carried out to fix.
        """
        assert CALIBRATION["combined_fpr"] >= max(CALIBRATION["per_corpus_fpr"].values())


class TestTheBinaryCannotSilentlyKillATrace:
    """M1: only FileNotFoundError was caught, and it is not the common case."""

    def test_a_non_executable_binary_is_reported_not_raised(self, tmp_path):
        """A COPY without +x in a Dockerfile is the classic way to hit this.

        PermissionError is an OSError sibling, so it escaped the handler and
        took `process()` down with it. The trace then carried a structural
        finding and no statistical property at all: not even "not assessed".
        """
        noexec = tmp_path / "stegcore"
        noexec.write_bytes(b"#!/bin/sh\nexit 0\n")
        noexec.chmod(0o644)
        with pytest.raises(StegcoreUnavailable, match="could not be run"):
            run(str(noexec), "/dev/null")

    def test_a_directory_in_place_of_the_binary(self, tmp_path):
        with pytest.raises(StegcoreUnavailable, match="could not be run"):
            run(str(tmp_path), "/dev/null")

    def test_a_missing_binary_still_reports(self, tmp_path):
        with pytest.raises(StegcoreUnavailable):
            run(str(tmp_path / "absent"), "/dev/null")

    def test_preflight_names_the_two_failures_apart(self, tmp_path):
        missing = tmp_path / "absent"
        with pytest.raises(StegcoreUnavailable, match="no analyser"):
            preflight(str(missing))

        noexec = tmp_path / "stegcore"
        noexec.write_bytes(b"x")
        noexec.chmod(0o644)
        with pytest.raises(StegcoreUnavailable, match="not executable"):
            preflight(str(noexec))

        noexec.chmod(0o755)
        preflight(str(noexec))  # must not raise

    def test_the_error_does_not_carry_a_temp_path_or_an_unbounded_stderr(self):
        """Evidence fields are not a place for our scratch paths.

        A failing analyser used to put `/tmp/tmpXXXX.png` and the whole of its
        stderr into `stegStatistical`, which is both meaningless to an
        examiner and unbounded.
        """
        import subprocess
        from unittest import mock

        long_stderr = (b"boom " * 10_000)
        completed = subprocess.CompletedProcess(
            args=[], returncode=1, stdout=b"", stderr=long_stderr
        )
        with mock.patch("subprocess.run", return_value=completed):
            with pytest.raises(StegcoreUnavailable) as caught:
                run("/opt/stegcore/stegcore", "/tmp/tmpsecret12345.png")
        message = str(caught.value)
        assert "tmpsecret12345" not in message
        assert len(message) < 400
        assert "truncated" in message


class TestAudioIsOutOfRange:
    """M6: an image corpus FPR must not be stapled to an audio measurement."""

    def test_audio_formats_are_not_declared_in_range(self):
        assert "wav" not in SPATIAL_FORMATS
        assert "flac" not in SPATIAL_FORMATS

    def test_every_declared_format_was_in_the_calibration_corpora(self):
        """Cassavia, BOSSbase and ALASKA2 are all image corpora.

        If a format is added here, there has to be a calibration that covers
        it, otherwise the plugin writes a false positive rate beside a
        measurement that rate was never established for.
        """
        assert SPATIAL_FORMATS <= {"png", "bmp", "tiff"}

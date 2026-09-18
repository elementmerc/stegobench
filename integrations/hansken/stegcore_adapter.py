# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Run Stegcore's analysis ensemble and turn it into findings an examiner can use.

WHY THIS SHELLS OUT INSTEAD OF REIMPLEMENTING THE DETECTORS
-----------------------------------------------------------
The thresholds below were calibrated against *Stegcore's* numbers, on named
corpora, at a measured false positive rate. A Python reimplementation of
Sample Pair Analysis would produce a slightly different number, and the
threshold would no longer mean what its calibration says it means. That is the
single-corpus mistake in a different costume: a threshold that has been moved
away from the thing it was measured against is a guess.

So the binary is the detector and this module is a translator.

WHAT IT REFUSES TO SAY, WHICH MATTERS MORE THAN WHAT IT SAYS
-------------------------------------------------------------
**Outside its validated range, this reports "not assessed", never "clean".**
Stegcore's statistical detectors model LSB replacement in the spatial domain.
Against JPEG-domain embedding they sit at chance, by construction. A tool used
outside its stated range produces a low score for the same reason an unplugged
microphone produces silence, and writing that into a trace as a negative
finding would let an examiner read "we looked and found nothing" off a
measurement that never looked.

**Chi-squared and LSB entropy are read but never used for a verdict.** On the
reference fixtures they return 0.868 and 0.997 on the clean picture and 0.868
and 0.997 on the one carrying a 0.4 bpp payload: identical to three decimals,
carrying no information about the thing being asked. They are reported for
completeness and excluded from the decision on that evidence.
"""
from __future__ import annotations

import dataclasses
import json
import os
import subprocess

#: Where the calibration came from and what it costs. Every one of these
#: numbers is in the record at `Stegcore/private/calibration/recal-final.json`
#: and is quoted in Stegcore's own operating rules; none of them was chosen by
#: eye. `fpr` is the combined false positive rate the whole ensemble holds on
#: the *worst* clean sub-distribution, which is the number that matters. A
#: threshold that holds 0% on one corpus and 22% on another has not been
#: calibrated, it has been fitted.
CALIBRATION = {
    "corpora": ["Cassavia 2022", "BOSSbase 1.01", "ALASKA2 (5.6k cover sample)"],
    "date": "2026-06-14",
    "combined_fpr": 0.04,
    "per_corpus_fpr": {"ALASKA2": 0.04, "Cassavia": 0.0, "BOSSbase": 0.001},
    "thresholds": {
        "Sample Pair Analysis": 0.3769769227919943,
        "RS Analysis": 0.30526622463808484,
        "Weighted Stego": 0.19485149015075318,
    },
}

#: Read from the output and reported, but never allowed to decide anything.
#: See the module docstring for the measurement behind this.
NON_DECIDING = ("Chi-Squared", "LSB Entropy")

#: Formats the statistical ensemble is validated against. A JPEG reaching this
#: module gets its structural result and an explicit "not assessed" for the
#: statistical one.
#:
#: **WAV and FLAC are deliberately absent.** The binary will happily analyse
#: them, but Cassavia 2022, BOSSbase 1.01 and ALASKA2 are all *image* corpora
#: and no audio was in any of them. Listing audio here would staple an image
#: corpus false positive rate to an audio measurement, which is the same
#: error, applying a threshold away from the distribution it was measured on,
#: that the module docstring above complains about. They go back in when there
#: is an audio calibration to point at.
SPATIAL_FORMATS = frozenset({"png", "bmp", "tiff"})

#: Truncate the analyser's error output before it reaches a trace. Nothing
#: downstream bounds it, and an examiner does not need a megabyte of somebody
#: else's stack trace in an evidence field.
MAX_STDERR_CHARS = 300

#: A picture should take a fraction of a second. This is the ceiling before we
#: decide the subprocess is not coming back, so that one pathological file
#: cannot stall an extraction.
TIMEOUT_SECONDS = 120


class StegcoreUnavailable(Exception):
    """The binary could not be run, so nothing was measured.

    Distinct from a clean result on purpose. An examiner must be able to tell
    "we looked and found nothing" from "we could not look".
    """


@dataclasses.dataclass(frozen=True)
class Detector:
    name: str
    score: float
    threshold: float
    detail: str

    @property
    def fires(self) -> bool:
        return self.score > self.threshold


@dataclasses.dataclass(frozen=True)
class Analysis:
    format: str
    #: None when the format is outside the ensemble's validated range.
    detectors: tuple[Detector, ...] | None
    fingerprint: str | None
    fingerprint_tier: str | None
    overall_score: float
    verdict: str
    reported: dict[str, float]

    @property
    def assessed(self) -> bool:
        return self.detectors is not None

    @property
    def firing(self) -> tuple[Detector, ...]:
        return tuple(d for d in self.detectors or () if d.fires)


def run(binary: str, path: str) -> Analysis:
    """Analyse one file. Raises rather than returning a false negative."""
    try:
        proc = subprocess.run(
            [binary, "analyse", "--json", path],
            capture_output=True,
            timeout=TIMEOUT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        raise StegcoreUnavailable(
            f"stegcore did not finish within {TIMEOUT_SECONDS} seconds"
        ) from exc
    except OSError as exc:
        # OSError, not FileNotFoundError. A binary copied into an image
        # without the execute bit, or a path that resolves to a directory,
        # raises PermissionError instead, which is a sibling and used to
        # escape this handler entirely and kill the whole trace: no
        # structural finding, no "not assessed", nothing.
        raise StegcoreUnavailable(f"the stegcore binary at {binary!r} could not be run: {exc.strerror}") from exc

    if proc.returncode != 0:
        stderr = proc.stderr.decode("utf-8", "replace").strip()
        if len(stderr) > MAX_STDERR_CHARS:
            stderr = stderr[:MAX_STDERR_CHARS] + " (truncated)"
        # The path is deliberately not quoted here: it is a temporary file
        # this plugin created, it means nothing to an examiner, and it ends up
        # written into evidence.
        raise StegcoreUnavailable(
            f"stegcore exited {proc.returncode}: {stderr or 'no error output'}"
        )

    return parse(proc.stdout)


def preflight(binary: str) -> None:
    """Check the binary can be run before any evidence is processed.

    Baseline rule: pre-flight everything. Discovering that the analyser is not
    executable on the first exhibit of a run, rather than at start up, means
    every trace up to that point carries a failure that looked like a result.
    """
    if not os.path.isfile(binary):
        raise StegcoreUnavailable(f"no analyser at {binary!r}")
    if not os.access(binary, os.X_OK):
        raise StegcoreUnavailable(f"the analyser at {binary!r} is not executable")


def parse(stdout: bytes) -> Analysis:
    """Turn one `stegcore analyse --json` document into an Analysis.

    Split from `run` so the mapping can be tested against captured output
    without the binary present, which is the pattern the rest of the harness
    already uses for shelled-out tools.
    """
    try:
        doc = json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise StegcoreUnavailable(f"stegcore produced output that is not JSON: {exc}") from exc

    if not doc.get("ok"):
        raise StegcoreUnavailable("stegcore reported the analysis did not succeed")

    entries = doc.get("data") or []
    if len(entries) != 1:
        raise StegcoreUnavailable(f"expected one analysis entry, got {len(entries)}")
    entry = entries[0]

    fmt = entry.get("format", "unknown")
    reported = {t["name"]: float(t["score"]) for t in entry.get("tests", [])}

    detectors: tuple[Detector, ...] | None = None
    if fmt in SPATIAL_FORMATS:
        found = []
        for test in entry.get("tests", []):
            name = test["name"]
            if name in NON_DECIDING:
                continue
            threshold = CALIBRATION["thresholds"].get(name)
            if threshold is None:
                # A detector the calibration does not cover cannot be given a
                # verdict, and silently dropping it would hide that.
                raise StegcoreUnavailable(
                    f"stegcore reported a detector this plugin has no calibrated "
                    f"threshold for: {name!r}"
                )
            found.append(
                Detector(
                    name=name,
                    score=float(test["score"]),
                    threshold=threshold,
                    detail=str(test.get("detail", "")),
                )
            )
        if not found:
            raise StegcoreUnavailable("stegcore reported no calibrated detectors")
        detectors = tuple(found)

    return Analysis(
        format=fmt,
        detectors=detectors,
        fingerprint=entry.get("tool_fingerprint"),
        fingerprint_tier=entry.get("tool_fingerprint_tier"),
        overall_score=float(entry.get("overall_score", 0.0)),
        verdict=str(entry.get("verdict", "unknown")),
        reported=reported,
    )

# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""A Hansken extraction plugin that looks for hidden data in pictures.

WHAT IT REPORTS, AND WHY IT IS TWO SEPARATE ANSWERS
----------------------------------------------------
Every picture it sees gets up to two findings, written to different
properties, because they are different kinds of claim:

**Structural.** Bytes sitting after the point where the file format says the
picture ends. No threshold, no corpus, no error rate, because nothing is being
estimated: either the bytes are there or they are not. This runs on every
picture and needs nothing installed.

**Statistical.** Stegcore's calibrated ensemble, which estimates whether the
low bits of the picture have been rewritten. This carries a false positive
rate and it only runs where that rate has been measured.

An examiner reading a trace needs to know which of the two they are holding,
so the plugin never merges them into one score.

THE THING IT DOES THAT OTHER TOOLS DO NOT
------------------------------------------
**On a JPEG it reports "not assessed", not "clean".**

Stegcore's statistical detectors model bit-level changes to pixels. JPEG
steganography does not change pixels, it changes coefficients, so against it
the detectors sit at chance, by construction and as measured. A low score
there means the question was never asked.

Almost every tool in this field reports that as a negative result, and a
negative result is what ends an investigation. Writing "not assessed" into the
trace instead is a small change that keeps a line of enquiry open, and it is
the reason this plugin is worth installing next to the ones that already
exist.

The structural check is unaffected, so a JPEG with something stuck on the end
is still caught.
"""
from __future__ import annotations

import os

from hansken_extraction_plugin.api.extraction_plugin import ExtractionPlugin
from hansken_extraction_plugin.api.plugin_info import (
    Author,
    MaturityLevel,
    PluginId,
    PluginInfo,
    PluginResources,
)
from hansken_extraction_plugin.runtime.extraction_plugin_runner import run_with_hanskenpy
from logbook import Logger

import stegcore_adapter
import trailing_data

log = Logger(__name__)

#: Where the analysis binary lives inside the plugin image. Overridable so the
#: test framework can run without it and a packager can move it.
STEGCORE_BINARY = os.environ.get("STEGCORE_BINARY", "/opt/stegcore/stegcore")

#: What the plugin declares to Hansken. This is the budget for the WHOLE
#: plugin, not per worker, which is the mistake the first version made.
MEMORY_BUDGET_MB = 2048
MAX_WORKERS = 2

#: Peak resident memory the analyser reaches, as a multiple of the file it is
#: given. **Measured, not estimated**: a 48 MB PNG drove the child to 555 MB,
#: which is 11.5x, and the worker additionally holds the bytes once and the
#: staged copy once. 14x is that with a little headroom.
ANALYSER_PEAK_RATIO = 14

#: Refuse to load a picture larger than this. Hansken hands over whatever is in
#: the evidence, including files that claim to be pictures and are not, and an
#: unbounded read is how one exhibit takes down a worker.
#:
#: Derived from the declaration above rather than picked, because picking it
#: was wrong by an order of magnitude: 256 MB against a 512 MB declaration and
#: four workers, when one worker on a fifth of that ceiling already exceeded
#: the whole budget on its own. A declaration a plugin cannot honour is worse
#: than no declaration, because the platform schedules against it.
MAX_PICTURE_BYTES = (MEMORY_BUDGET_MB // MAX_WORKERS // ANALYSER_PEAK_RATIO) * 1024 * 1024

#: Where findings are written. `misc` is the namespace Hansken's own examples
#: use for properties outside the core schema, which is what these are until
#: there is a schema to put them in.
NS = "picture.misc"


class SteganographyPlugin(ExtractionPlugin):
    def plugin_info(self):
        return PluginInfo(
            id=PluginId(domain="stegcore.dev", category="picture", name="SteganographyDetection"),
            version="0.1.0",
            description=(
                "Looks for hidden data in pictures: data appended after the end of the "
                "picture (exact), and rewritten low bits (calibrated, spatial formats "
                "only). Reports 'not assessed' rather than 'clean' where the statistical "
                "detectors are outside their validated range."
            ),
            author=Author("Daniel Iwugo", "daniel@themalwarefiles.com", "The Malware Files"),
            maturity=MaturityLevel.PROOF_OF_CONCEPT,
            webpage_url="https://github.com/The-Malware-Files/Stegcore",
            matcher="type=picture AND $data.type=raw",
            license="AGPL-3.0-or-later",
            resources=PluginResources(
                maximum_cpu=MAX_WORKERS,
                maximum_memory=MEMORY_BUDGET_MB,
                maximum_workers=MAX_WORKERS,
            ),
        )

    def __init__(self):
        super().__init__()
        # Pre-flight once, not per exhibit. The result is remembered so a
        # missing or non-executable analyser is reported identically on every
        # trace instead of being rediscovered, and so a long extraction does
        # not pay for the check on each picture.
        try:
            stegcore_adapter.preflight(STEGCORE_BINARY)
            self._analyser_problem: str | None = None
        except stegcore_adapter.StegcoreUnavailable as exc:
            log.warning(f"the analyser is unusable, statistical findings will be skipped: {exc}")
            self._analyser_problem = str(exc)

    def process(self, trace, data_context):
        name = str(trace.get("file.name") or trace.get("name") or "<unnamed>")
        size = data_context.data_size

        if size > MAX_PICTURE_BYTES:
            # Both properties, not just a skip marker. A trace carrying neither
            # is invisible to a query for what has not been assessed, which is
            # the query an examiner runs to find the gaps in their own case.
            reason = f"not assessed: {size} bytes exceeds the {MAX_PICTURE_BYTES} byte ceiling"
            log.warning(f"{name}: {reason}")
            trace.update({f"{NS}.stegStructural": reason, f"{NS}.stegStatistical": reason})
            return

        with trace.open() as reader:
            data = reader.read(size)

        self._structural(trace, name, data)
        self._statistical(trace, name, data)

    def _structural(self, trace, name, data):
        try:
            found = trailing_data.find_trailing(data)
        except trailing_data.MalformedImage as exc:
            # A file that does not parse has not been cleared, and saying so
            # is the whole point of separating this from a clean result.
            log.info(f"{name}: does not parse, so nothing is claimed about it ({exc})")
            trace.update(f"{NS}.stegStructural", f"not assessed: {exc}")
            return
        except trailing_data.PrependedData as exc:
            log.info(f"{name}: {exc}")
            trace.update(
                {
                    f"{NS}.stegStructural": "prepended data",
                    f"{NS}.stegPrependedBytes": str(exc.offset),
                }
            )
            return
        except ValueError:
            trace.update(f"{NS}.stegStructural", "not assessed: not a PNG or JPEG")
            return

        if not found.present:
            trace.update(f"{NS}.stegStructural", "none")
            return

        described = found.looks_like or "unrecognised data"
        log.info(f"{name}: {found.length} bytes after the picture ends ({described})")
        trace.update(
            {
                f"{NS}.stegStructural": "appended data",
                f"{NS}.stegAppendedBytes": str(found.length),
                f"{NS}.stegAppendedOffset": str(found.offset),
                f"{NS}.stegAppendedLooksLike": described,
            }
        )

    def _statistical(self, trace, name, data):
        if self._analyser_problem is not None:
            trace.update(f"{NS}.stegStatistical", f"not assessed: {self._analyser_problem}")
            return
        try:
            analysis = self._analyse(name, data)
        except stegcore_adapter.StegcoreUnavailable as exc:
            # Degrade visibly. The structural finding above still stands, and
            # an examiner must not read a missing detector as a clean picture.
            log.warning(f"{name}: statistical analysis unavailable ({exc})")
            trace.update(f"{NS}.stegStatistical", f"not assessed: {exc}")
            return

        if not analysis.assessed:
            # No calibration note here on purpose: quoting a false positive
            # rate beside a measurement that was never taken invites a reader
            # to believe one was.
            trace.update(
                f"{NS}.stegStatistical",
                f"not assessed: the calibrated detectors are validated for "
                f"{', '.join(sorted(stegcore_adapter.SPATIAL_FORMATS))} and this is "
                f"{analysis.format}",
            )
            return

        firing = analysis.firing
        updates = {
            f"{NS}.stegStatistical": "above threshold" if firing else "below threshold",
            f"{NS}.stegCalibration": self._calibration_note(),
        }
        for det in analysis.detectors:
            key = det.name.replace(" ", "")
            updates[f"{NS}.steg{key}"] = f"{det.score:.6f} (threshold {det.threshold:.6f})"
        if firing:
            updates[f"{NS}.stegDetectorsFiring"] = ", ".join(d.name for d in firing)
        if analysis.fingerprint:
            updates[f"{NS}.stegToolFingerprint"] = analysis.fingerprint
            updates[f"{NS}.stegToolFingerprintTier"] = analysis.fingerprint_tier or "unknown"

        log.info(f"{name}: {len(firing)} of {len(analysis.detectors)} detectors above threshold")
        trace.update(updates)

    def _analyse(self, name, data):
        """Hand the bytes to the binary.

        The binary takes a path, and Hansken hands over a stream, so the data
        is staged. Written and removed inside one call so nothing outlives the
        trace it came from.
        """
        import tempfile

        with tempfile.NamedTemporaryFile(suffix=_suffix(name), delete=True) as tmp:
            tmp.write(data)
            tmp.flush()
            return stegcore_adapter.run(STEGCORE_BINARY, tmp.name)

    @staticmethod
    def _calibration_note():
        c = stegcore_adapter.CALIBRATION
        return (
            f"thresholds hold {c['combined_fpr']:.0%} combined false positive rate, "
            f"calibrated {c['date']} on {', '.join(c['corpora'])}"
        )


def _suffix(name: object) -> str:
    """The extension the analyser needs to pick a parser, never the caller's path.

    `name` is whatever the trace carried, which is not guaranteed to be a
    string: a numeric filename came back as an int and `name.lower()` took the
    whole of `process()` down with it, after the structural finding had already
    been written. The trace then held half an answer and no error.
    """
    lowered = str(name).lower()
    for ext in (".png", ".jpg", ".jpeg", ".bmp", ".tiff"):
        if lowered.endswith(ext):
            return ext
    return ".png"


if __name__ == "__main__":
    run_with_hanskenpy(SteganographyPlugin)

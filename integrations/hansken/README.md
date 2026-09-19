<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Steganography detection: a Hansken extraction plugin

Finds hidden data in pictures, and is careful about the difference between
finding nothing and not having looked.

Status: **proof of concept.** 63 tests, plus 9 more through the Hansken SDK's
own framework when a build of the analyser is present. It has not been run
inside Hansken itself, and the image has not been built.

## What it writes to a trace

Two findings, deliberately kept apart, under `picture.misc`.

| Property | Meaning |
|---|---|
| `stegStructural` | `none`, `appended data`, or `not assessed: <reason>` |
| `stegAppendedBytes` / `stegAppendedOffset` / `stegAppendedLooksLike` | Where the extra data starts, how much there is, and what it appears to be |
| `stegStatistical` | `below threshold`, `above threshold`, or `not assessed: <reason>` |
| `stegSamplePairAnalysis` / `stegRSAnalysis` / `stegWeightedStego` | Each score against the threshold it is being judged by |
| `stegDetectorsFiring` | Which detectors crossed, when any did |
| `stegCalibration` | The false positive rate those thresholds hold, the date, and the corpora |
| `stegPrependedBytes` | How many bytes sit in front of the picture, when the signature is not at offset zero |
| `stegToolFingerprint` / `stegToolFingerprintTier` | A named embedding tool, and whether that match is exact or heuristic |

`stegCalibration` is written only where a calibrated measurement was actually
taken. It is deliberately absent from a `not assessed` trace, because quoting a
false positive rate beside a measurement nobody made invites a reader to
believe one was.

### Why two findings and not one score

They are different kinds of claim.

**Structural** means the file format says the picture ends here and there are
bytes after it. There is no threshold and no error rate, because nothing is
estimated. Either the bytes are there or they are not.

**Statistical** means a model of what an untouched picture looks like was
compared against a threshold. It can be wrong on a picture that is merely
unusual, and it carries a measured false positive rate.

Averaging those into a single number would hide which one an examiner is
holding.

## The part that is the point

**On a JPEG this reports `not assessed`, never `clean`.**

The statistical detectors model bit-level changes to pixels. JPEG
steganography does not change pixels, it changes the coefficients the picture
is compressed into, so against it these detectors sit at chance. A low score
there is not evidence of absence; it is the question never having been asked.

Most tools in this field report that as a negative result, and a negative
result is what closes a line of enquiry. The structural check still runs, so a
JPEG with something stuck on the end is still caught.

The same applies when the analysis binary cannot be run: the structural
finding stands, and the statistical property says why it is missing rather
than going quiet. That covers a missing file, a file without the execute bit
and a path that is a directory. Only the first of those used to be handled;
the other two raised out of `process()` and left the trace with no statistical
property at all, which is the one outcome this design exists to prevent. The
check now runs once at start up rather than per exhibit.

## Where the thresholds come from

They were calibrated on 2026-06-14 against three corpora: Cassavia 2022,
BOSSbase 1.01, and a 5,600 image cover sample from ALASKA2.

| Detector | Threshold |
|---|---|
| Sample Pair Analysis | 0.3769769227919943 |
| RS Analysis | 0.30526622463808484 |
| Weighted Stego | 0.19485149015075318 |

They hold a **4% combined false positive rate on the worst of the three**
(ALASKA2 4%, Cassavia 0.0%, BOSSbase 0.1%). The worst one is the number
quoted, on purpose: an earlier set calibrated on the other two alone leaked
around 22% on ALASKA2's JPEG-decompressed covers, which is what a threshold
fitted to one distribution does when it meets another.

Two of the five detectors the binary reports, chi-squared and LSB entropy, are
read and recorded but never allowed to decide anything. On the reference
fixtures they return the same value to three decimals on a clean picture and
on one carrying a 0.4 bpp payload. A test pins that, so if they ever start
carrying information the exclusion gets revisited rather than inherited.

## Known blind spots, stated rather than discovered

- **JPEG-domain embedding**: not covered, and reported as such.
- **Audio is not covered either.** The binary will analyse WAV and FLAC, but
  Cassavia, BOSSbase and ALASKA2 are all image corpora, so the thresholds and
  the 4% figure mean nothing there. Audio formats are deliberately absent from
  the in-range list rather than silently inheriting an image corpus error rate.
- **Low payload**: below about 0.1 bpp, detection is weak. This is the ceiling
  of classical detectors, not a defect of this one.
- **LSB matching** (plus or minus one) rather than LSB replacement: weak, for
  the same reason.
- The appended-data fingerprint is **heuristic** tier, which is why the tier
  is written to the trace instead of being left implicit.

## Running the tests

```
tox                                    # 63 tests, no analyser needed
STEGCORE_BINARY=/path/to/stegcore tox -e sdk    # the SDK's own framework
```

`tox` runs pytest, which covers both modes through a stand-in trace: with the
analyser and without it. It needs nothing installed.

**The SDK framework is a separate environment on purpose.** Its golden traces
in `testdata/result/` were recorded with the analyser present, so on a machine
without a build of Stegcore it fails 4 of 9. An earlier version had it in the
default run and claimed in this file that "the suite still runs" without the
binary. It did not; that was false for everyone but the author, and the
degraded mode this README calls the interesting one had no test at all.

The fixtures are a clean PNG, the same PNG with 4,096 bytes appended, a PNG
carrying a 0.4 bpp LSB payload, and a clean JPEG. They are byte identical on
every machine, generated by `generators/make_fixtures.py` one level up.

## Building the image

```
tox -e package
```

The first stage builds the analysis binary from a pinned commit of Stegcore,
and every base image is pinned by digest. Tags move; digests are the content.

Two things in that file used to be untrue and are worth stating plainly. The
"commit" was `22d05afa`, which is the annotated v4.1.0 *tag object*, not a
commit; it is now the commit it dereferences to, `d179ddfd`. And a comment
claimed the builder's Rust version matched `rust-toolchain.toml` at that
commit, which cannot be so because that file did not exist until later. The
toolchain choice is this image's own and nothing upstream confirms it.

## Licence

AGPL-3.0-or-later, matching Stegcore, whose analysis this wraps.

**Two files are dual licensed**: `trailing_data.py` and its tests are
`AGPL-3.0-or-later OR Apache-2.0`. They are original work containing no
Stegcore code and depending on nothing but the standard library, so their
licence is separable from the rest of this tree. The second licence exists so
the structural detector can be contributed to collections that cannot accept
copyleft, NFI's own plugin examples among them. `LICENSE-APACHE-2.0` holds
that text, byte identical to the copy in their repository.

Everything else here, and everything it touches in Stegcore, stays AGPL. The
statistical half is where the calibration and the thresholds live and it is
not offered under a permissive licence.

`nfi-example/` holds a standalone Apache-2.0 copy shaped for their examples
repository. Nothing there has been sent.

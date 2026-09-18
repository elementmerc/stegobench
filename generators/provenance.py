#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""What a cover carries with it: how it was compressed, and where it was cut from.

These are acquisition facts rather than image-quality facts, and they are shared
by every fetcher, so they live here rather than inside one source's module.

WHY THE JPEG PROFILE IS RECORDED
--------------------------------
81.5% of Wikimedia Commons is JPEG, so most covers in this corpus have already
been through a lossy codec before we ever see them. That is not a defect to
apologise for, it is the normal condition of real imagery, but it is a variable
that has to be *measured* because it moves results more than most embedding
parameters do.

Sedighi and Fridrich measured the consequence directly: the ranking of embedding
schemes **inverts** between never-compressed covers and decompressed JPEGs, with
WOW the least secure scheme on BOSSbase and the most secure on decompressed
JPEGs. A corpus that mixes both without recording which is which cannot tell a
reader which regime a number belongs to.

The quantisation table is the honest fingerprint of that history. It is the
actual matrix the encoder divided by, it survives in the file, and two images
sharing a table went through the same encoder settings. Recording its digest
lets a user group the corpus by compression history without trusting anybody's
quality estimate, and the estimated quality is offered beside it as a convenience
rather than as the truth.

WHY THE CROP POSITION IS RANDOM
-------------------------------
Every cover so far was cut from the centre of its source image. Photographers
put their subject in the middle, so a centre crop systematically over-samples
subjects and under-samples background: skies, walls, foliage, water. Those are
exactly the smooth regions where adaptive embedding refuses to put changes and
where detection is hardest, so a centre-cropped corpus is quietly biased towards
the easy case.

The position is drawn from a seed derived from the image's own identifier, so it
is reproducible from the manifest without storing anything extra, and two runs
over the same source produce the same corpus.
"""
from __future__ import annotations

import hashlib
import random

# The luminance quantisation table the JPEG standard gives as its example, and
# which every IJG-derived encoder scales. Quality estimation works by scaling
# this the way the encoder would and finding which quality reproduces what we
# actually observe.
IJG_LUMINANCE = (
    16, 11, 10, 16, 24, 40, 51, 61,
    12, 12, 14, 19, 26, 58, 60, 55,
    14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62,
    18, 22, 37, 56, 68, 109, 103, 77,
    24, 35, 55, 64, 81, 104, 113, 92,
    49, 64, 78, 87, 103, 121, 120, 101,
    72, 92, 95, 98, 112, 100, 103, 99,
)

# ISO is recorded in stops rather than as a raw number because the thing it
# stands in for, sensor noise, roughly doubles with each stop. Bands make the
# axis something a quota can balance; the raw value stays in the manifest.
ISO_BANDS = ((0, 200, "base"), (200, 800, "low"), (800, 3200, "high"),
             (3200, 10**9, "extreme"))


def scale_for_quality(quality: int) -> float:
    """The IJG scale factor for a quality setting, as libjpeg computes it."""
    quality = max(1, min(100, quality))
    return 5000 / quality if quality < 50 else 200 - 2 * quality


def table_for_quality(quality: int) -> tuple[int, ...]:
    scale = scale_for_quality(quality)
    return tuple(
        max(1, min(255, int((v * scale + 50) / 100))) for v in IJG_LUMINANCE
    )


def estimate_quality(table) -> int | None:
    """Which IJG quality setting best explains this quantisation table.

    An estimate and labelled as one. An encoder that is not IJG-derived, or one
    using a custom table, will still produce a number here and that number will
    mean less than it appears to. The table digest beside it is the fact; this
    is the convenience.
    """
    values = list(table)[:64]
    if len(values) < 64:
        return None
    best, best_error = None, None
    for quality in range(1, 101):
        candidate = table_for_quality(quality)
        error = sum((a - b) ** 2 for a, b in zip(values, candidate))
        if best_error is None or error < best_error:
            best, best_error = quality, error
    return best


def jpeg_profile(img) -> dict:
    """The compression history of an already-opened image, as far as it survives.

    Returns an empty profile for anything that is not a JPEG, which is the
    honest answer: a PNG or TIFF original has no quantisation history, and that
    absence is itself the interesting property. See `pristine`.
    """
    tables = getattr(img, "quantization", None)
    if not tables:
        return {"compressed": False}

    digest = hashlib.sha256()
    for key in sorted(tables):
        digest.update(bytes(str(key), "ascii"))
        digest.update(bytes(bytearray(int(v) & 0xFF for v in tables[key])))

    luminance = tables.get(0)
    return {
        "compressed": True,
        "quant_tables": len(tables),
        "quant_digest": digest.hexdigest()[:16],
        "estimated_quality": estimate_quality(luminance) if luminance else None,
        "progressive": bool(getattr(img, "info", {}).get("progressive")),
        "subsampling": _subsampling(img),
    }


def _subsampling(img) -> str | None:
    """Chroma subsampling as a plain label, since Pillow reports it as an int."""
    try:
        from PIL import JpegImagePlugin
        value = JpegImagePlugin.get_sampling(img)
    except Exception:  # noqa: BLE001 - absent on non-JPEG, and not worth raising
        return None
    return {0: "4:4:4", 1: "4:2:2", 2: "4:2:0"}.get(value)


def pristine(mime: str | None, profile: dict) -> bool:
    """True when nothing lossy has touched this image before we did.

    A never-compressed cover is rare and disproportionately valuable: it is the
    regime in which almost every published steganalysis result was obtained, and
    the one a corpus drawn from the web can barely supply. Commons holds a few
    per cent as TIFF and PNG originals, which is worth separating rather than
    diluting.

    It is a claim about the file we received, not a guarantee about its whole
    history. A PNG re-encoded from a JPEG carries no quantisation table and will
    read as pristine here. The mime type and the absent table are the only
    evidence available, and the manifest records both so a reader can disagree.
    """
    return mime in ("image/png", "image/tiff") and not profile.get("compressed")


def iso_band(exif: dict) -> str | None:
    """Which noise regime this frame was shot in, or None if it never said."""
    raw = exif.get("ISOSpeedRatings")
    if raw in (None, ""):
        return None
    try:
        value = int(str(raw).split()[0].strip("[](),"))
    except (ValueError, IndexError):
        return None
    for low, high, name in ISO_BANDS:
        if low <= value < high:
            return name
    return None


def crop_box(width: int, height: int, size: int, seed: str) -> tuple[int, int, int, int]:
    """Where to cut a `size` square out of a `width` by `height` image.

    Reproducible from the seed, which callers derive from the image's own
    identifier, so the manifest does not have to store the position for a
    rebuild to land in the same place.
    """
    if width < size or height < size:
        raise ValueError(f"{width}x{height} is smaller than the {size}px crop")
    rng = random.Random(hashlib.sha256(seed.encode("utf-8")).hexdigest())
    left = rng.randint(0, width - size)
    top = rng.randint(0, height - size)
    return left, top, left + size, top + size

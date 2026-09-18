#!/usr/bin/env python3
"""Fetch cover images from Wikimedia Commons, with per-file licence provenance.

Commons is the cleanest large source available to this project. Its acceptance
policy is the reason: a file cannot be on Commons at all unless "Republication
and distribution must be allowed", "Publication of derivative work must be
allowed" and "Commercial use of the work must be allowed". So the two terms that
rule out almost every academic steganalysis corpus, non-commercial and
no-derivatives, cannot exist here by construction.

It also needs no API key, which matters: the Pexels route is in doubt because
that site's Terms of Service ban bulk automated collection regardless of what its
licence permits, and Unsplash needs a key we do not have.

WHAT IS RECORDED, AND WHY ALL OF IT
-----------------------------------
Every cover carries its Commons page title and id, the exact licence short name,
the artist and credit strings, the original source URL, the original dimensions,
Commons' own sha1 of the original file, and a sha256 of the crop we produced.

The licence is per file and is not decoration. Commons guarantees derivatives and
commercial use are permitted; it does NOT guarantee they are unconditional. A
CC BY-SA file obliges any derived work to carry CC BY-SA too, and a stego image
is a derived work. Publishing a corpus that mixes share-alike and permissive
files without recording which is which makes that obligation impossible to
honour afterwards. Hence `--licences`, defaulting to the permissive set only.

WHAT COUNTS AS A CANDIDATE
--------------------------
Two sampling strategies, and the default is `random` for a measured reason. The
curated featured and quality picture categories reach an aesthetically selected
slice of Commons. Uniform random draws over the file namespace reach what it
actually holds, which is largely digitised books, engravings and maps, because
bulk archive uploads dominate the public domain half. Neither is a corpus of
photographs on its own.

So a candidate must also carry a camera make in its EXIF, which is the cheapest
available proof that a camera produced it, and which hands the corpus its
acquisition-diversity axis at the same time. Pass `--allow-scans` to lift that,
and give the scans their own arm rather than mixing them in.

Every candidate that survives is then put through the cover suitability gate and
the shared deduplication store, in that order, because a flat crop must never
reach the store: a picture with no gradients hashes its own rounding noise and
collides with every other flat crop. Both kinds of refusal are written to
`rejected.jsonl` beside the manifest rather than dropped.

CROP, NEVER RESIZE, AND NEVER FROM THE CENTRE
---------------------------------------------
Resampling averages neighbouring pixels, which rewrites the whole
least-significant-bit plane: exactly the statistic spatial steganalysis reads.
Commons offers thumbnail URLs at any width and they are useless to us for that
reason. We take the original bytes and cut a square out of them.

Where we cut matters as much as that we cut. Photographers compose with the
subject near the middle, so a centre crop systematically over-samples subjects
and under-samples background: sky, wall, foliage, water. Those smooth regions
are where adaptive embedding refuses to spend its payload and where detection is
hardest, so a centre-cropped corpus is quietly weighted towards the easy case.
The position is drawn from a seed made of the page id, so it is reproducible
from the manifest without storing anything extra.

WHAT ELSE EACH COVER CARRIES
----------------------------
The compression history, because 81.5% of Commons is JPEG and the ranking of
embedding schemes is known to invert between never-compressed covers and
decompressed ones. The ISO band, because sensor noise roughly doubles per stop
and is what a steganalyser is reading underneath the payload. Whether the file
was ever lossily compressed at all, since the never-compressed regime is the one
most published results were obtained in and the hardest to source from the web.
See `provenance.py` for all three.

POLITENESS
----------
Commons is donated infrastructure. Featured images routinely run to tens of
megabytes and downloading 80 MB to keep a quarter of a megapixel is rude as well
as slow, so candidates are filtered on the file size the API reports BEFORE any
download. There is a delay between requests and a descriptive User-Agent, which
is what their robot policy asks for.
"""
import argparse
import hashlib
import io
import itertools
import json
import pathlib
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

from PIL import Image

import cover_quality
import provenance
from dedup import DedupError, DedupStore, fingerprint_bytes
from manifest_repair import (attribution_for, capture_class, stable_split,
                             undouble, LICENCE_URLS)

API = "https://commons.wikimedia.org/w/api.php"
UA = "stegobench/0.1 (steganalysis research corpus; https://github.com/elementmerc/stegobench)"

# Categories chosen for subject diversity rather than prettiness. A corpus of
# four hundred photographs of the same thing measures that thing.
DEFAULT_CATEGORIES = [
    "Category:Featured pictures of landscapes",
    "Category:Featured pictures of animals",
    "Category:Featured pictures of plants",
    "Category:Featured pictures of architecture",
    "Category:Featured pictures of people",
    "Category:Featured pictures of vehicles",
    "Category:Featured pictures of food",
    "Category:Quality images of street art",
    "Category:Quality images of interiors",
    "Category:Quality images of night photographs",
]

# Licences that permit derivative redistribution with no share-alike obligation.
# CC BY-SA and the GFDL-ish tags are deliberately NOT here by default: they are
# usable, but they infect the derived corpus with their own terms.
PERMISSIVE = {
    "cc0", "cc0 1.0", "public domain", "pd", "pd-us", "pd-old",
    "cc by 1.0", "cc by 2.0", "cc by 2.5", "cc by 3.0", "cc by 4.0",
}


def api_get(params: dict, retries: int = 4) -> dict:
    """One API call with backoff. Commons answers JSON and rate-limits politely."""
    last = "no attempt"
    for attempt in range(1, retries + 1):
        url = f"{API}?{urllib.parse.urlencode(params)}"
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        try:
            with urllib.request.urlopen(req, timeout=45) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            if e.code == 429:
                wait = 30 * attempt
                print(f"  rate limited, waiting {wait}s", file=sys.stderr)
                time.sleep(wait)
                last = "429"
                continue
            last = f"HTTP {e.code}"
        except Exception as e:  # noqa: BLE001 - the message is the useful part
            last = str(e)
        time.sleep(3 * attempt)
    raise RuntimeError(f"commons API failed after {retries} attempts: {last}")


def fetch_bytes(url: str, retries: int = 3) -> bytes:
    last = "no attempt"
    for attempt in range(1, retries + 1):
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                return r.read()
        except Exception as e:  # noqa: BLE001
            last = str(e)
            time.sleep(3 * attempt)
    raise RuntimeError(f"download failed after {retries} attempts: {last}")


def strip_html(value: str) -> str:
    """Commons returns artist and credit as HTML fragments. Keep the text.

    The result is passed through `undouble` because Commons embeds a hidden
    microformat copy of the same text inside the fragment, and a tag stripper
    concatenates both. That produced 96 rows reading "Unknown authorUnknown
    author" in the 10,000 cover corpus, corrupting exactly the field a CC BY
    user has to rely on. Found by a hostile review on 2026-09-18.
    """
    out, depth = [], 0
    for ch in value or "":
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth = max(0, depth - 1)
        elif depth == 0:
            out.append(ch)
    return undouble(" ".join("".join(out).split()))


def centre_crop(img: Image.Image, size: int) -> Image.Image:
    w, h = img.size
    if w < size or h < size:
        raise ValueError(f"{w}x{h} is smaller than the {size}px crop")
    left, top = (w - size) // 2, (h - size) // 2
    return img.crop((left, top, left + size, top + size))


IIPROP = "url|size|mime|sha1|user|extmetadata|metadata"
IIEXTFILTER = "LicenseShortName|Artist|Credit|UsageTerms"

# EXIF fields worth keeping, and only these. Camera make and model give the
# corpus the acquisition-diversity axis REVEAL built its reputation on, and the
# exposure triple explains the sensor noise a steganalyser is reading: a high
# ISO frame carries far more of it than a base ISO one, and that difference
# moves detection results more than most embedding parameters do.
EXIF_KEEP = ("Make", "Model", "ISOSpeedRatings", "ExposureTime", "FNumber",
             "FocalLength", "DateTimeOriginal")


def extmetadata_of(ii: dict) -> dict:
    """The extmetadata block, or an empty one if the API did not send a block.

    The MediaWiki API does not guarantee the shape of this field. For most files
    it is an object keyed by property name; for some it arrives as an empty
    *list* instead, which is JSON's other way of spelling "nothing here" and
    which has no `.get`. An overnight fetch died on its eighth cover for exactly
    that reason.

    This is a boundary, so the coercion belongs here rather than at every use.
    """
    em = ii.get("extmetadata")
    return em if isinstance(em, dict) else {}


def extmeta_value(em: dict, key: str) -> str:
    """One extmetadata string, tolerating every shape the API sends."""
    field = em.get(key)
    if isinstance(field, dict):
        value = field.get("value")
    else:
        value = field
    return "" if value is None else str(value).strip()


def exif_of(ii: dict) -> dict:
    """The EXIF fields we keep, flattened out of the API's list-of-dicts shape."""
    entries = ii.get("metadata")
    if not isinstance(entries, list):
        return {}
    raw = {m.get("name"): m.get("value")
           for m in entries if isinstance(m, dict)}
    out = {}
    for key in EXIF_KEEP:
        value = raw.get(key)
        if value not in (None, ""):
            out[key] = str(value).strip()
    return out


class DiversityCaps:
    """Limits on how much of the corpus any one uploader or camera may be.

    WHY A CAP AND NOT A HASH
    ------------------------
    Deduplication answers "is this the same picture?" and answers it well. It
    has nothing to say about "is this the four hundredth photograph of the same
    subject, from the same camera, by the same uploader?", because every one of
    those frames genuinely is a different picture and a perceptual hash is right
    to admit them.

    That case is not hypothetical. A 20 cover test run on 2026-09-16 returned
    four frames of `ISS0xx-E-xxxxx - View of Earth`, all shot on one Nikon D4
    aboard the space station, because NASA has uploaded tens of thousands of
    them and uniform random sampling weights files rather than photographers.

    Commons hands us the uploader and the camera in the same API response we are
    already reading, so the cap costs nothing and is exactly reproducible.
    Counting resumes from the manifest, so an interrupted run does not reset the
    tally and quietly double every cap.
    """

    def __init__(self, per_uploader: int, per_camera: int) -> None:
        self.per_uploader = per_uploader
        self.per_camera = per_camera
        self.uploaders: dict[str, int] = {}
        self.cameras: dict[str, int] = {}

    @staticmethod
    def camera_key(exif: dict) -> str | None:
        make, model = exif.get("Make", ""), exif.get("Model", "")
        key = f"{make} {model}".strip()
        return key or None

    def resume_from(self, rows: list) -> None:
        for row in rows:
            self.record(row.get("uploader"), self.camera_key(row.get("exif") or {}))

    def refusal(self, uploader: str | None, camera: str | None) -> str | None:
        """Why this candidate is refused, or None if there is room for it."""
        if uploader and self.per_uploader > 0:
            if self.uploaders.get(uploader, 0) >= self.per_uploader:
                return (f"uploader {uploader} has already contributed "
                        f"{self.per_uploader} covers")
        if camera and self.per_camera > 0:
            if self.cameras.get(camera, 0) >= self.per_camera:
                return (f"camera {camera} has already contributed "
                        f"{self.per_camera} covers")
        return None

    def record(self, uploader: str | None, camera: str | None) -> None:
        if uploader:
            self.uploaders[uploader] = self.uploaders.get(uploader, 0) + 1
        if camera:
            self.cameras[camera] = self.cameras.get(camera, 0) + 1


class ShareQuota:
    """No single value on an axis may take more than a share of the corpus.

    A hard cap is the wrong instrument for an axis whose values are not equally
    available. Base ISO frames outnumber high ISO ones heavily in any photo
    collection, so capping them at a fixed count would either be so high it
    never fires or so low the fetch stalls waiting for grain that never comes.

    A share quota says instead: whatever the corpus turns out to be, no one
    value may exceed this fraction of it. The corpus stays balanced without the
    fetch ever blocking on a stratum the source cannot supply.

    The floor exists because a share is meaningless on a small sample. Before it
    is reached nothing is refused, otherwise the first cover of a run would be
    100% of one band and every later one would be turned away.
    """

    def __init__(self, axis: str, max_share: float, floor: int = 200) -> None:
        if not 0 < max_share <= 1:
            raise ValueError(f"max_share must be in (0, 1], got {max_share}")
        self.axis = axis
        self.max_share = max_share
        self.floor = floor
        self.counts: dict[str, int] = {}
        self.total = 0

    def resume_from(self, values) -> None:
        for value in values:
            self.record(value)

    def refusal(self, value: str | None) -> str | None:
        if value is None or self.total < self.floor:
            return None
        held = self.counts.get(value, 0)
        if held + 1 > self.max_share * (self.total + 1):
            return (f"{self.axis} {value} already holds {held} of {self.total} "
                    f"covers, over the {self.max_share:.0%} share")
        return None

    def record(self, value: str | None) -> None:
        self.total += 1
        if value is not None:
            self.counts[value] = self.counts.get(value, 0) + 1


def suitable(ii: dict, min_kb: int, max_kb: int, size: int) -> bool:
    """Everything decidable from API metadata, before a byte is downloaded.

    Commons is donated infrastructure and its featured images reach 80 MB, so
    anything we can rule out from the listing is ruled out there.
    """
    # TIFF is here for one reason: it is where the never-compressed originals
    # are. A few per cent of Commons is TIFF, those files have no quantisation
    # history, and that regime is the one almost every published steganalysis
    # result was obtained in. See provenance.pristine.
    if not ii or ii.get("mime") not in ("image/jpeg", "image/png", "image/tiff"):
        return False
    kb = ii.get("size", 0) // 1024
    if not (min_kb <= kb <= max_kb):
        return False
    return min(ii.get("width", 0), ii.get("height", 0)) >= size


def candidates(category: str, limit: int, min_kb: int, max_kb: int, size: int):
    """Yield files in `category` whose reported size and dimensions suit us."""
    cont = {}
    seen = 0
    while seen < limit:
        params = {
            "action": "query", "format": "json", "formatversion": "2",
            "generator": "categorymembers", "gcmtitle": category,
            "gcmtype": "file", "gcmlimit": "100",
            "prop": "imageinfo",
            "iiprop": IIPROP, "iiextmetadatafilter": IIEXTFILTER,
        }
        params.update(cont)
        try:
            d = api_get(params)
        except RuntimeError as e:
            print(f"  {category}: {e}", file=sys.stderr)
            return
        for page in d.get("query", {}).get("pages", []):
            ii = (page.get("imageinfo") or [{}])[0]
            if not suitable(ii, min_kb, max_kb, size):
                continue
            yield page, ii
            seen += 1
            if seen >= limit:
                return
        cont = d.get("continue") or {}
        if not cont:
            return


def random_candidates(limit: int, min_kb: int, max_kb: int, size: int,
                      delay: float):
    """Yield uniformly random Commons files that pass the metadata filter.

    WHY RANDOM RATHER THAN CURATED CATEGORIES
    -----------------------------------------
    The category route reaches "featured" and "quality" pictures, which is an
    aesthetically selected slice: competent photographers, good light, flattering
    subjects, and a camera population skewed towards expensive bodies. A corpus
    built from it measures detection on prize-winning photographs.

    Uniform random sampling of the file namespace reaches what Commons actually
    holds. Measured on 200 random draws on 2026-09-16: 52% are both at least
    512px on each side and permissively licensed, so roughly two draws buy one
    cover. 47.5% carry camera EXIF, and the makes include Apple, Xiaomi, Samsung
    and Google alongside Canon and Nikon, which is closer to the imagery a
    deployed detector meets than any curated set would be.

    The generator caps out at 20 files per call, so this is the slow half of the
    pipeline and the reason `--delay` exists.
    """
    seen = 0
    misses = 0
    while seen < limit:
        try:
            d = api_get({
                "action": "query", "format": "json", "formatversion": "2",
                "generator": "random", "grnnamespace": "6", "grnlimit": "20",
                "prop": "imageinfo",
                "iiprop": IIPROP, "iiextmetadatafilter": IIEXTFILTER,
            })
        except RuntimeError as e:
            print(f"  random sampling: {e}", file=sys.stderr)
            return
        got = 0
        for page in d.get("query", {}).get("pages", []):
            ii = (page.get("imageinfo") or [{}])[0]
            if not suitable(ii, min_kb, max_kb, size):
                continue
            got += 1
            yield page, ii
            seen += 1
            if seen >= limit:
                return
        # A run of empty draws means the filter is rejecting everything, which
        # is a misconfiguration rather than bad luck. Say so instead of looping.
        misses = misses + 1 if got == 0 else 0
        if misses >= 20:
            print("  random sampling: 400 consecutive files failed the size and "
                  "type filter. Check --min-kb, --max-kb and --size.",
                  file=sys.stderr)
            return
        time.sleep(delay)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--count", type=int, default=400, help="total covers wanted")
    ap.add_argument("--size", type=int, default=512, help="square crop side")
    ap.add_argument("--categories", default=",".join(DEFAULT_CATEGORIES))
    ap.add_argument("--min-kb", type=int, default=300,
                    help="skip files smaller than this; tiny files are often graphics")
    ap.add_argument("--max-kb", type=int, default=6000,
                    help="skip files larger than this; Commons featured images "
                         "reach 80 MB and we keep a quarter of a megapixel")
    ap.add_argument("--licences", default="permissive",
                    choices=("permissive", "any"),
                    help="permissive: CC0, public domain and plain CC BY only, so "
                         "the derived corpus carries no share-alike obligation. "
                         "any: also accept CC BY-SA, which obliges the derived "
                         "work to be share-alike too")
    ap.add_argument("--delay", type=float, default=1.0,
                    help="seconds between downloads; Commons is donated "
                         "infrastructure, and it answered 429 at 0.4s spacing "
                         "when this was measured")
    ap.add_argument("--strategy", default="random", choices=("random", "categories"),
                    help="random: uniform draws from the file namespace, which is "
                         "what Commons actually holds. categories: the curated "
                         "featured and quality picture sets, which are an "
                         "aesthetically selected slice")
    ap.add_argument("--require-exif", action="store_true", default=True,
                    help="keep only files carrying a camera make, which is proof "
                         "a camera produced them (default)")
    ap.add_argument("--allow-scans", dest="require_exif", action="store_false",
                    help="also accept files with no camera EXIF. On Commons that "
                         "is mostly digitised books, engravings, maps and "
                         "diagrams, whose statistics are nothing like a "
                         "photograph's. Their own arm, never mixed into one")
    ap.add_argument("--max-per-uploader", type=int, default=40,
                    help="most covers one Commons uploader may contribute. Bulk "
                         "importers such as the NASA feeds would otherwise take "
                         "a visible share of a random sample. 0 disables")
    ap.add_argument("--max-per-camera", type=int, default=40,
                    help="most covers one camera make and model may contribute, "
                         "which is the acquisition-diversity axis this corpus "
                         "sells. 0 disables")
    ap.add_argument("--max-iso-share", type=float, default=0.5,
                    help="largest share of the corpus any one ISO band may take. "
                         "Sensor noise roughly doubles per stop and dominates "
                         "detection, so a corpus that is nearly all base ISO "
                         "measures one noise regime and calls it steganalysis")
    ap.add_argument("--split-salt", default="pentimento-v1",
                    help="fixed and recorded. Changing it after publication "
                         "reassigns every cover's train/test split, which "
                         "silently invalidates every result built on the corpus")
    ap.add_argument("--test-fraction", type=float, default=0.2)
    ap.add_argument("--dedup-db", default=None,
                    help="path to the shared dedup store. Without it this fetcher "
                         "cannot tell that a cover already arrived from another "
                         "source or an earlier session, so it is strongly advised")
    args = ap.parse_args()

    # This runs detached overnight with its output redirected to a file, and
    # Python block-buffers stdout when it is not a terminal. A heartbeat sitting
    # in an 8 KB buffer is not a heartbeat, so ask for line buffering explicitly.
    sys.stdout.reconfigure(line_buffering=True)

    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest_path = out / "manifest.jsonl"
    rejected_path = out / "rejected.jsonl"

    caps = DiversityCaps(args.max_per_uploader, args.max_per_camera)
    iso_quota = ShareQuota("iso band", args.max_iso_share)
    seen_ids = set()
    if manifest_path.exists():
        rows = [json.loads(l) for l in manifest_path.read_text().splitlines() if l.strip()]
        seen_ids = {r["pageid"] for r in rows}
        caps.resume_from(rows)
        iso_quota.resume_from(r.get("iso_band") for r in rows)
        print(f"resuming: {len(seen_ids)} covers already fetched")

    store = None
    if args.dedup_db:
        try:
            store = DedupStore(args.dedup_db)
        except DedupError as e:
            print(f"dedup store: {e}", file=sys.stderr)
            return 2
        print(f"dedup store holds {store.count()} images")
    else:
        print("no --dedup-db given: duplicates across sources will NOT be caught",
              file=sys.stderr)

    written = len(seen_ids)
    skipped = {"licence": 0, "not_a_photograph": 0, "over_cap": 0,
               "over_quota": 0, "error": 0,
               "download": 0, "decode": 0, "too_small": 0, "flat": 0,
               "duplicate": 0}
    last_beat = time.monotonic()

    if args.strategy == "random":
        # The budget must cover the measured yield with room to spare. A random
        # draw that is a permissively licensed photograph large enough to crop
        # runs at about 22%, so a cover costs roughly 4.5 candidates and a
        # budget of 4 per cover stops the run at a fraction of what was asked
        # for while reporting success. 40 leaves an order of magnitude of
        # headroom for a source whose composition drifts; the consecutive-miss
        # guard inside the generator is what actually stops a runaway.
        source = random_candidates(args.count * 40, args.min_kb, args.max_kb,
                                   args.size, args.delay)
    else:
        cats = [c.strip() for c in args.categories.split(",") if c.strip()]
        per_cat = max(1, args.count // len(cats))
        source = itertools.chain.from_iterable(
            candidates(c, per_cat * 4, args.min_kb, args.max_kb, args.size)
            for c in cats
        )

    with manifest_path.open("a") as mf, rejected_path.open("a") as rf:
        for page, ii in source:
            if written >= args.count:
                break
            if page["pageid"] in seen_ids:
                continue
            # This run is measured in hours and one malformed API record has
            # already killed it once. A candidate that cannot be processed now
            # costs itself and nothing more, and names itself on the way out.
            try:
                em = extmetadata_of(ii)
                lic = extmeta_value(em, "LicenseShortName")
                if args.licences == "permissive" and lic.lower() not in PERMISSIVE:
                    skipped["licence"] += 1
                    continue

                exif = exif_of(ii)
                # A camera make is the cheapest available proof that a camera made
                # this file. Commons' public domain holdings are dominated by bulk
                # archive digitisation, so uniform random sampling without this
                # filter returns mostly scanned books: measured 2026-09-16, 78 of
                # 130 permissively licensed files carried no camera EXIF at all.
                if args.require_exif and not exif.get("Make"):
                    skipped["not_a_photograph"] += 1
                    continue

                uploader = (ii.get("user") or "").strip() or None
                camera = DiversityCaps.camera_key(exif)
                # Checked before the download, because a cover we will not keep is
                # bandwidth taken from donated infrastructure for nothing.
                capped = caps.refusal(uploader, camera)
                if capped:
                    skipped["over_cap"] += 1
                    rf.write(json.dumps({
                        "pageid": page["pageid"], "title": page["title"],
                        "source_url": ii["url"], "reason": "diversity_cap",
                        "detail": capped, "uploader": uploader, "camera": camera,
                    }) + "\n")
                    rf.flush()
                    continue

                try:
                    raw = fetch_bytes(ii["url"])
                except RuntimeError:
                    skipped["download"] += 1
                    continue
                try:
                    img = Image.open(io.BytesIO(raw))
                    img.load()
                    # Read the compression history off the file as opened. The
                    # quantisation tables live on the decoder and a convert()
                    # discards them, so this cannot be deferred.
                    profile = provenance.jpeg_profile(img)
                    img = img.convert("L") if img.mode in ("L", "I;16", "I") else img.convert("RGB")
                except Exception:  # noqa: BLE001
                    skipped["decode"] += 1
                    continue
                try:
                    # Random rather than centre: photographers put the subject in
                    # the middle, so a centre crop over-samples subjects and
                    # under-samples the smooth background where adaptive
                    # embedding hides and detection is hardest. Seeded from the
                    # page id, so a rebuild cuts in the same place.
                    box = provenance.crop_box(
                        img.size[0], img.size[1], args.size,
                        f"commons:{page['pageid']}",
                    )
                    cropped = img.crop(box)
                except ValueError:
                    skipped["too_small"] += 1
                    continue

                seen_ids.add(page["pageid"])

                band = provenance.iso_band(exif)
                crowded = iso_quota.refusal(band)
                if crowded:
                    skipped["over_quota"] += 1
                    rf.write(json.dumps({
                        "pageid": page["pageid"], "title": page["title"],
                        "source_url": ii["url"], "reason": "share_quota",
                        "detail": crowded, "iso_band": band,
                    }) + "\n")
                    rf.flush()
                    continue

                # Suitability before uniqueness, deliberately: a flat crop must never
                # reach the dedup store, because a picture with no gradients hashes
                # its own rounding noise and collides with every other flat crop.
                quality = cover_quality.assess(cropped)
                if not quality.usable:
                    skipped["flat"] += 1
                    measured = quality.as_dict()
                    rf.write(json.dumps({
                        "pageid": page["pageid"], "title": page["title"],
                        "source_url": ii["url"],
                        # The code is what a later pass groups on, so it must not be
                        # overwritten by the prose explaining it.
                        "reason": "unusable_cover",
                        "detail": quality.reason,
                        "texture": measured["texture"],
                        "clipped": measured["clipped"],
                    }) + "\n")
                    rf.flush()
                    continue

                # Encode once. The bytes fingerprinted are the bytes written, so the
                # manifest digest, the dedup store and the file on disk cannot drift.
                # Rebuild from raw pixels so nothing rides along. Pillow carries
                # the source's ancillary data through a crop, and 207 of the first
                # 400 covers inherited an ICC profile from their originals while
                # 193 did not. A corpus where half the clean covers carry a
                # variable-size ancillary chunk has an uncontrolled variable in
                # it, and the structural arm is specifically about data appended
                # to a file, so this is a confound rather than untidiness. One
                # profile was also large enough to trip Pillow's decompression
                # guard on re-open, which is the error that surfaced it.
                bare = Image.frombytes(cropped.mode, cropped.size, cropped.tobytes())
                buf = io.BytesIO()
                bare.save(buf, format="PNG", optimize=False)
                payload = buf.getvalue()
                digest = hashlib.sha256(payload).hexdigest()

                if store is not None:
                    decision = store.offer(
                        fingerprint_bytes(payload), "commons", str(page["pageid"])
                    )
                    if not decision.accepted:
                        skipped["duplicate"] += 1
                        rf.write(json.dumps({
                            "pageid": page["pageid"], "title": page["title"],
                            "source_url": ii["url"], "reason": "duplicate",
                            "detail": decision.match.describe(),
                            "matched_sha256": decision.match.sha256,
                            "match_kind": decision.match.kind,
                            "match_distance": decision.match.distance,
                        }) + "\n")
                        rf.flush()
                        continue

                name = f"{written:05d}.png"
                path = out / name
                # Atomic: a killed run leaves no half-written PNG behind for the
                # next one to read as a finished cover.
                part = path.with_suffix(".png.part")
                part.write_bytes(payload)
                part.replace(path)

                record = {
                    "file": name,
                    "pageid": page["pageid"],
                    "title": page["title"],
                    "source_url": ii["url"],
                    "descriptionurl": ii.get("descriptionurl"),
                    "licence": lic,
                    "usage_terms": strip_html(extmeta_value(em, "UsageTerms")),
                    "artist": strip_html(extmeta_value(em, "Artist")),
                    "credit": strip_html(extmeta_value(em, "Credit")),
                    "original_size": [ii.get("width"), ii.get("height")],
                    "original_bytes": ii.get("size"),
                    "original_mime": ii.get("mime"),
                    "commons_sha1": ii.get("sha1"),
                    "exif": exif,
                    "uploader": uploader,
                    "iso_band": band,
                    "compression": profile,
                    "pristine": provenance.pristine(ii.get("mime"), profile),
                    "crop_box": list(box),
                    "strategy": args.strategy,
                    "crop": args.size,
                    "method": "random_crop",
                    "mode": cropped.mode,
                    "quality": quality.as_dict(),
                    "sha256": digest,
                }
                # Fields a downstream user has to act on, derived here so they
                # cannot drift from the row they describe. `tier_order` is
                # deliberately NOT among them: a tier is a prefix of an ordering
                # over the whole corpus, which no single fetch can know, so
                # manifest_repair.py assigns it once the set is complete.
                record["licence_url"] = LICENCE_URLS.get((lic or "").strip())
                record["capture_class"], record["capture_class_basis"] = \
                    capture_class(record)
                record["attribution"], record["attribution_required"] = \
                    attribution_for(record)
                record["split"] = stable_split(record, args.split_salt,
                                               args.test_fraction)
                record["split_salt"] = args.split_salt
                mf.write(json.dumps(record) + "\n")
                mf.flush()
                caps.record(uploader, camera)
                iso_quota.record(band)
                written += 1

                if time.monotonic() - last_beat >= 60:
                    print(f"  ... {written}/{args.count} covers, skipped {skipped}",
                          flush=True)
                    last_beat = time.monotonic()
                time.sleep(args.delay)
            except Exception as e:  # noqa: BLE001 - one bad record, not one dead run
                skipped["error"] += 1
                seen_ids.add(page["pageid"])
                print(f"  skipping {page.get('title', page['pageid'])}: "
                      f"{type(e).__name__}: {e}", file=sys.stderr)
                rf.write(json.dumps({
                    "pageid": page["pageid"], "title": page.get("title"),
                    "reason": "error", "detail": f"{type(e).__name__}: {e}",
                }) + "\n")
                rf.flush()
                continue


    if store is not None:
        store.close()
    print(f"wrote {written} covers of {args.count} requested into {out}")
    if any(skipped.values()):
        print(f"  skipped: {skipped}")
    if written < args.count:
        # Short of the target is a result, not a detail. A caller that reads
        # "wrote 4000" as success builds an arm a quarter of the size it
        # documents, which is the kind of thing nobody notices until the paper.
        print(f"  SHORT by {args.count - written}: the candidate supply ran out "
              f"before the target was met. Re-run the same command to continue, "
              f"or loosen --min-kb, --max-kb or the diversity caps.",
              file=sys.stderr)
    print(f"  manifest: {manifest_path}")
    if rejected_path.exists() and rejected_path.stat().st_size:
        print(f"  rejections: {rejected_path}")
    return 0 if written else 1


if __name__ == "__main__":
    raise SystemExit(main())

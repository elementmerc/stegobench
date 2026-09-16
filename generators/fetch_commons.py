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

CROP, NEVER RESIZE
------------------
Resampling averages neighbouring pixels, which rewrites the whole
least-significant-bit plane: exactly the statistic spatial steganalysis reads.
Commons offers thumbnail URLs at any width and they are useless to us for that
reason. We take the original bytes and centre-crop.

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
from dedup import DedupError, DedupStore, fingerprint_bytes

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
    """Commons returns artist and credit as HTML fragments. Keep the text."""
    out, depth = [], 0
    for ch in value or "":
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth = max(0, depth - 1)
        elif depth == 0:
            out.append(ch)
    return " ".join("".join(out).split())


def centre_crop(img: Image.Image, size: int) -> Image.Image:
    w, h = img.size
    if w < size or h < size:
        raise ValueError(f"{w}x{h} is smaller than the {size}px crop")
    left, top = (w - size) // 2, (h - size) // 2
    return img.crop((left, top, left + size, top + size))


IIPROP = "url|size|mime|sha1|extmetadata|metadata"
IIEXTFILTER = "LicenseShortName|Artist|Credit|UsageTerms"

# EXIF fields worth keeping, and only these. Camera make and model give the
# corpus the acquisition-diversity axis REVEAL built its reputation on, and the
# exposure triple explains the sensor noise a steganalyser is reading: a high
# ISO frame carries far more of it than a base ISO one, and that difference
# moves detection results more than most embedding parameters do.
EXIF_KEEP = ("Make", "Model", "ISOSpeedRatings", "ExposureTime", "FNumber",
             "FocalLength", "DateTimeOriginal")


def exif_of(ii: dict) -> dict:
    """The EXIF fields we keep, flattened out of the API's list-of-dicts shape."""
    raw = {m.get("name"): m.get("value") for m in (ii.get("metadata") or [])}
    out = {}
    for key in EXIF_KEEP:
        value = raw.get(key)
        if value not in (None, ""):
            out[key] = str(value).strip()
    return out


def suitable(ii: dict, min_kb: int, max_kb: int, size: int) -> bool:
    """Everything decidable from API metadata, before a byte is downloaded.

    Commons is donated infrastructure and its featured images reach 80 MB, so
    anything we can rule out from the listing is ruled out there.
    """
    if not ii or ii.get("mime") not in ("image/jpeg", "image/png"):
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

    seen_ids = set()
    if manifest_path.exists():
        for line in manifest_path.read_text().splitlines():
            if line.strip():
                seen_ids.add(json.loads(line)["pageid"])
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
    skipped = {"licence": 0, "not_a_photograph": 0, "download": 0, "decode": 0,
               "too_small": 0, "flat": 0, "duplicate": 0}
    last_beat = time.monotonic()

    if args.strategy == "random":
        source = random_candidates(args.count * 4, args.min_kb, args.max_kb,
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
            em = ii.get("extmetadata", {})
            lic = (em.get("LicenseShortName", {}).get("value") or "").strip()
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

            try:
                raw = fetch_bytes(ii["url"])
            except RuntimeError:
                skipped["download"] += 1
                continue
            try:
                img = Image.open(io.BytesIO(raw))
                img = img.convert("L") if img.mode in ("L", "I;16", "I") else img.convert("RGB")
            except Exception:  # noqa: BLE001
                skipped["decode"] += 1
                continue
            try:
                cropped = centre_crop(img, args.size)
            except ValueError:
                skipped["too_small"] += 1
                continue

            seen_ids.add(page["pageid"])

            # Suitability before uniqueness, deliberately: a flat crop must never
            # reach the dedup store, because a picture with no gradients hashes
            # its own rounding noise and collides with every other flat crop.
            quality = cover_quality.assess(cropped)
            if not quality.usable:
                skipped["flat"] += 1
                rf.write(json.dumps({
                    "pageid": page["pageid"], "title": page["title"],
                    "source_url": ii["url"], "reason": "unusable_cover",
                    "detail": quality.reason, **quality.as_dict(),
                }) + "\n")
                rf.flush()
                continue

            # Encode once. The bytes fingerprinted are the bytes written, so the
            # manifest digest, the dedup store and the file on disk cannot drift.
            buf = io.BytesIO()
            cropped.save(buf, format="PNG", optimize=False)
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

            mf.write(json.dumps({
                "file": name,
                "pageid": page["pageid"],
                "title": page["title"],
                "source_url": ii["url"],
                "descriptionurl": ii.get("descriptionurl"),
                "licence": lic,
                "usage_terms": strip_html(em.get("UsageTerms", {}).get("value", "")),
                "artist": strip_html(em.get("Artist", {}).get("value", "")),
                "credit": strip_html(em.get("Credit", {}).get("value", "")),
                "original_size": [ii.get("width"), ii.get("height")],
                "original_bytes": ii.get("size"),
                "original_mime": ii.get("mime"),
                "commons_sha1": ii.get("sha1"),
                "exif": exif,
                "strategy": args.strategy,
                "crop": args.size,
                "method": "centre_crop",
                "mode": cropped.mode,
                "quality": quality.as_dict(),
                "sha256": digest,
            }) + "\n")
            mf.flush()
            written += 1

            if time.monotonic() - last_beat >= 60:
                print(f"  ... {written}/{args.count} covers, skipped {skipped}",
                      flush=True)
                last_beat = time.monotonic()
            time.sleep(args.delay)

    if store is not None:
        store.close()
    print(f"wrote {written} covers of {args.count} requested into {out}")
    if any(skipped.values()):
        print(f"  skipped: {skipped}")
    print(f"  manifest: {manifest_path}")
    if rejected_path.exists() and rejected_path.stat().st_size:
        print(f"  rejections: {rejected_path}")
    return 0 if written else 1


if __name__ == "__main__":
    raise SystemExit(main())

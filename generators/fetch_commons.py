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
import json
import pathlib
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

from PIL import Image

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


def candidates(category: str, limit: int, min_kb: int, max_kb: int, size: int):
    """Yield files in `category` whose reported size and dimensions suit us.

    Filtering happens on API metadata, before any image is downloaded.
    """
    cont = {}
    seen = 0
    while seen < limit:
        params = {
            "action": "query", "format": "json", "formatversion": "2",
            "generator": "categorymembers", "gcmtitle": category,
            "gcmtype": "file", "gcmlimit": "100",
            "prop": "imageinfo",
            "iiprop": "url|size|mime|sha1|extmetadata",
            "iiextmetadatafilter": "LicenseShortName|Artist|Credit|UsageTerms",
        }
        params.update(cont)
        try:
            d = api_get(params)
        except RuntimeError as e:
            print(f"  {category}: {e}", file=sys.stderr)
            return
        for page in d.get("query", {}).get("pages", []):
            ii = (page.get("imageinfo") or [{}])[0]
            if not ii or ii.get("mime") not in ("image/jpeg", "image/png"):
                continue
            kb = ii.get("size", 0) // 1024
            if not (min_kb <= kb <= max_kb):
                continue
            if min(ii.get("width", 0), ii.get("height", 0)) < size:
                continue
            yield page, ii
            seen += 1
            if seen >= limit:
                return
        cont = d.get("continue") or {}
        if not cont:
            return


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
                    help="seconds between downloads; Commons is donated infrastructure")
    args = ap.parse_args()

    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    manifest_path = out / "manifest.jsonl"

    seen_ids = set()
    if manifest_path.exists():
        for line in manifest_path.read_text().splitlines():
            if line.strip():
                seen_ids.add(json.loads(line)["pageid"])
        print(f"resuming: {len(seen_ids)} covers already fetched")

    cats = [c.strip() for c in args.categories.split(",") if c.strip()]
    per_cat = max(1, args.count // len(cats))
    written = len(seen_ids)
    skipped = {"licence": 0, "download": 0, "decode": 0, "too_small": 0}

    with manifest_path.open("a") as mf:
        for cat in cats:
            if written >= args.count:
                break
            got = 0
            for page, ii in candidates(cat, per_cat * 4, args.min_kb, args.max_kb, args.size):
                if got >= per_cat or written >= args.count:
                    break
                if page["pageid"] in seen_ids:
                    continue
                em = ii.get("extmetadata", {})
                lic = (em.get("LicenseShortName", {}).get("value") or "").strip()
                if args.licences == "permissive" and lic.lower() not in PERMISSIVE:
                    skipped["licence"] += 1
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

                name = f"{written:05d}.png"
                path = out / name
                cropped.save(path)
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
                    "commons_sha1": ii.get("sha1"),
                    "category": cat,
                    "crop": args.size,
                    "method": "centre_crop",
                    "mode": cropped.mode,
                    "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                }) + "\n")
                mf.flush()
                seen_ids.add(page["pageid"])
                written += 1
                got += 1
                time.sleep(args.delay)

    print(f"wrote {written} covers of {args.count} requested into {out}")
    if any(skipped.values()):
        print(f"  skipped: {skipped}")
    print(f"  manifest: {manifest_path}")
    return 0 if written else 1


if __name__ == "__main__":
    raise SystemExit(main())

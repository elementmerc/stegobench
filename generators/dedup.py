#!/usr/bin/env python3
"""The deduplication store: one image may enter the corpus once, and only once.

WHY THIS IS A CORRECTNESS REQUIREMENT
-------------------------------------
A duplicate in a steganalysis corpus is not untidiness, it is a wrong number.
The same photograph appearing in both the training and the test half lets a
detector recognise the picture instead of the payload, and every accuracy figure
built on that corpus is inflated by an amount nobody can measure afterwards. It
is a documented defect in MIRFLICKR and it is invisible unless you look for it.

Three failure modes, and exact hashing catches only the first:

1. The identical file from two services. Heavily reposted images sit on both
   Unsplash and Commons. sha256 catches these.
2. Near duplicates inside one service. Burst frames, a crop, a recolour, a
   resave at a different quality. Every byte differs; the picture is the same.
3. Cross arm leakage, which is (1) or (2) discovered after the split has already
   been made, by which point it is too late.

So: sha256 as the cheap first pass, then two perceptual hashes.

WHY TWO PERCEPTUAL HASHES
-------------------------
They fail differently, which is the entire point of carrying both.

  dHash reads *gradients*: it shrinks the picture to 9x8 grey pixels and records,
  for each adjacent pair, which one is brighter. It survives brightness and
  contrast changes and resizes, because those keep the direction of a gradient
  even when they move its size. It is fooled by a picture with very little
  structure, where the gradients are noise and two unrelated flat images can
  agree by chance.

  pHash reads *frequencies*: it shrinks to 32x32, runs a discrete cosine
  transform (the same maths JPEG uses), and keeps the low frequency corner,
  which is the coarse shape of the picture with the fine detail discarded. It is
  the more robust of the two against crops and small rotations and the more
  expensive to compute.

A candidate is rejected when *either* hash says duplicate. Erring towards
rejection is deliberate: the cost of wrongly rejecting a cover is that we fetch
another one, and covers are abundant. The cost of wrongly accepting a duplicate
is a corpus with a silent defect in it.

The exception is a picture too thin to hash honestly, where both must agree. See
`CORROBORATION_BELOW`.

WHAT THIS STORE CANNOT DO, MEASURED RATHER THAN ASSUMED
-------------------------------------------------------
It catches *duplicates*. It does not catch *redundancy*, and no threshold
setting will make it.

The distinction matters because the two look alike in a corpus listing. A
duplicate is the same picture arriving twice. Redundancy is four hundred
genuinely different photographs of the same subject, from one camera, by one
uploader, which is what a bulk contributor such as the NASA station feeds
produces.

Measured on 2026-09-16 across 7 ISS "View of Earth" frames and 20 unrelated
Commons photographs, pHash distances were:

    within the ISS set          min 24, median 30
    within unrelated set        min 24, median 32
    ISS against unrelated       min 22, median 32

The distributions are the same. Two photographs of Earth from the space station
are no more alike, to a perceptual hash, than two photographs of unrelated
things. There is no threshold between them because there is no gap.

Reaching for them by raising the threshold fails for a second, independent
reason: chance collisions grow far faster than the radius does.

    radius   a random pair collides   at 10k held   at 100k   at 25M
       6            1 in 221 billion         0.000     0.000    0.000
       7             1 in 26 billion         0.000     0.000    0.001
      10            1 in 100 million         0.000     0.001    0.250
      15                 1 in 82,086         0.122     1.218  304.557

The right hand columns are expected false collisions per candidate. Distance 15
is survivable at ten thousand images and worthless at ten million, so a
threshold chosen today against a small corpus would quietly poison a large one.
There is headroom to about 10 if a measurement ever justifies it; there is none
at all in the range that would be needed here, and the ISS numbers above say the
range needed here is past 24 regardless.

Redundancy is therefore handled where the evidence for it actually lives, in the
acquisition metadata: see `DiversityCaps` in `fetch_commons.py`, which limits how
many covers one uploader or one camera body may contribute.

HOW THE LOOKUP STAYS FAST AT ANY SCALE
--------------------------------------
The naive near duplicate search compares the candidate against every hash held,
which means holding every hash in memory and doing linear work per candidate.
That is fine at ten thousand images and unacceptable at a hundred million, and
baseline Section 12 asks that today's choice not force tomorrow's rewrite.

Instead each 64 bit hash is cut into 8 bands of 8 bits and every band is indexed
in SQLite. Two hashes within Hamming distance 7 of each other must, by the
pigeonhole principle, agree *exactly* on at least one band: 8 differing bits
cannot be spread across 8 bands without leaving one untouched. So an indexed
lookup on the candidate's 8 bands returns a small superset of the true matches,
and only that superset is compared bit by bit. Nothing is ever held in memory
proportional to the corpus size.

The consequence of that arithmetic: MAX_SUPPORTED_DISTANCE is 7. A threshold
above that would silently start missing duplicates, so it is refused rather than
accepted.

WHAT GETS HASHED
----------------
The image as it enters the corpus, which is the crop we keep rather than the
original download. That is the honest choice: the corpus contains the crop, so
the corpus is deduplicated on the crop. Two different crops of one original are
genuinely different pictures to a detector, and two originals that crop to the
same picture are genuinely the same one.

REJECTIONS ARE RECORDED, NOT DISCARDED
--------------------------------------
Every rejection is written to the store with what it collided with and by how
much. A corpus that can show what it excluded and why is one a reviewer can
check; a corpus that silently drops candidates is one that has to be believed.
"""
from __future__ import annotations

import argparse
import contextlib
import dataclasses
import datetime as dt
import hashlib
import pathlib
import sqlite3
import sys
import time

import numpy as np
from PIL import Image

import cover_quality

SCHEMA_VERSION = 2

HASH_BITS = 64
BAND_COUNT = 8
BAND_BITS = HASH_BITS // BAND_COUNT
# Pigeonhole: BAND_COUNT bands can absorb at most BAND_COUNT - 1 differing bits
# while still leaving one band untouched. Above this the banded index stops
# being a superset of the true matches and starts losing duplicates silently.
MAX_SUPPORTED_DISTANCE = BAND_COUNT - 1

DEFAULT_DHASH_MAX = 6
DEFAULT_PHASH_MAX = 6

# Below this much local structure, one hash agreeing is not evidence and the
# other must agree too before a candidate is refused.
#
# Measured 2026-09-16 and this is not a precaution. Across 7 ISS Earth views and
# 20 unrelated Commons photographs, the closest cross pair in the whole set had
# dHash distance 6, right on the threshold, while pHash put the same pair at 28,
# which is as unrelated as two images get. Both had barely any texture: 1.00 and
# 1.43 grey levels, above `cover_quality.TEXTURE_FLOOR` and well inside its
# advisory band.
#
# The cause is structural rather than unlucky. dHash records, for each adjacent
# pair of pixels, which is brighter. On a picture with almost no gradients those
# comparisons are decided by rounding, so two unrelated flat images agree at
# roughly the rate two coin flips agree. pHash degenerates the same way for the
# same reason: with no energy outside the DC term, its 63 remaining bits are
# noise about a median of noise.
#
# So the rule is not "trust pHash instead". It is that a thin picture needs both
# hashes to agree, which two independent noise sources will not do.
CORROBORATION_BELOW = 4.0

# SQLite's default lock wait is zero, which turns any concurrent writer into an
# instant "database is locked". Baseline Section 2.1: every wait is deadline
# bounded, and a bounded wait that is long enough to be useful beats no wait.
DEFAULT_LOCK_TIMEOUT = 30.0

PIL_MAX_PIXELS = 120_000_000  # a decompression bomb guard, not a quality limit


class DedupError(RuntimeError):
    """Anything the caller can act on: a bad store, a bad image, a bad setting."""


@dataclasses.dataclass(frozen=True)
class Fingerprint:
    sha256: str
    dhash: int
    phash: int
    width: int
    height: int
    # Mean local structure, in grey levels. Carried because a hash computed on a
    # picture with almost no structure is a hash of its own rounding noise, and
    # the store has to know that before it convicts anything. See
    # `CORROBORATION_BELOW` and `find_match`.
    texture: float = 0.0

    def band(self, kind: str, idx: int) -> int:
        value = self.dhash if kind == "d" else self.phash
        return (value >> (idx * BAND_BITS)) & ((1 << BAND_BITS) - 1)


@dataclasses.dataclass(frozen=True)
class Match:
    """What a candidate collided with, and on which evidence."""
    sha256: str
    source: str
    ref: str
    kind: str          # "exact", "dhash" or "phash"
    distance: int      # 0 for an exact match

    def describe(self) -> str:
        if self.kind == "exact":
            return f"byte identical to {self.source}:{self.ref}"
        name = "gradient" if self.kind == "dhash" else "frequency"
        return (f"{name} hash within {self.distance} bits of "
                f"{self.source}:{self.ref}")


@dataclasses.dataclass(frozen=True)
class Decision:
    accepted: bool
    fingerprint: Fingerprint
    match: Match | None = None


def _dct_matrix(n: int) -> np.ndarray:
    """The DCT-II basis as a matrix, so the transform is two matrix multiplies.

    Built here rather than imported so the module needs numpy and Pillow only;
    scipy is present on atlas but not everywhere this has to run.
    """
    k = np.arange(n).reshape(-1, 1)
    x = np.arange(n).reshape(1, -1)
    m = np.cos(np.pi * (2 * x + 1) * k / (2 * n))
    m[0] *= 1 / np.sqrt(2)
    return m * np.sqrt(2 / n)


_DCT32 = _dct_matrix(32)


def _bits_to_int(bits: np.ndarray) -> int:
    """Pack a flat boolean array, most significant bit first, into one integer."""
    out = 0
    for bit in bits.astype(bool).ravel():
        out = (out << 1) | int(bit)
    return out


def dhash(img: Image.Image) -> int:
    """Gradient hash: is each pixel brighter than the one to its right?"""
    small = np.asarray(
        img.convert("L").resize((9, 8), Image.LANCZOS), dtype=np.int16
    )
    return _bits_to_int(small[:, 1:] > small[:, :-1])


def phash(img: Image.Image) -> int:
    """Frequency hash: the low frequency corner of the DCT, against its median."""
    small = np.asarray(
        img.convert("L").resize((32, 32), Image.LANCZOS), dtype=np.float64
    )
    freq = _DCT32 @ small @ _DCT32.T
    low = freq[:8, :8].ravel()
    # The DC term is the average brightness of the whole picture. It dwarfs
    # every other coefficient, so including it in the median would drag the
    # threshold and let a uniform brightness change move unrelated bits. It is
    # excluded from the median and its own bit is pinned to zero.
    bits = low > np.median(low[1:])
    bits[0] = False
    return _bits_to_int(bits)


def hamming(a: int, b: int) -> int:
    return (a ^ b).bit_count()


def fingerprint_bytes(raw: bytes) -> Fingerprint:
    """Fingerprint an image held in memory. Raises DedupError on anything unreadable."""
    if not raw:
        raise DedupError("empty image data")
    digest = hashlib.sha256(raw).hexdigest()
    previous = Image.MAX_IMAGE_PIXELS
    Image.MAX_IMAGE_PIXELS = PIL_MAX_PIXELS
    try:
        import io
        with Image.open(io.BytesIO(raw)) as img:
            img.load()
            width, height = img.size
            if width < 8 or height < 8:
                raise DedupError(f"{width}x{height} is too small to fingerprint")
            return Fingerprint(digest, dhash(img), phash(img), width, height,
                               cover_quality.texture(img))
    except DedupError:
        raise
    except Exception as e:  # noqa: BLE001 - the caller needs the reason, not the type
        raise DedupError(f"could not decode image: {e}") from e
    finally:
        Image.MAX_IMAGE_PIXELS = previous


def fingerprint_file(path: pathlib.Path) -> Fingerprint:
    path = pathlib.Path(path)
    try:
        raw = path.read_bytes()
    except OSError as e:
        raise DedupError(f"could not read {path}: {e}") from e
    try:
        return fingerprint_bytes(raw)
    except DedupError as e:
        raise DedupError(f"{path}: {e}") from e


class DedupStore:
    """A persistent, concurrent-safe record of every image the corpus has seen.

    Safe to open from several worker processes at once: SQLite in WAL mode
    serialises the writers and the one thing that must not race, the check and
    the insert, happens inside a single IMMEDIATE transaction.
    """

    def __init__(
        self,
        path: str | pathlib.Path,
        dhash_max: int = DEFAULT_DHASH_MAX,
        phash_max: int = DEFAULT_PHASH_MAX,
        lock_timeout: float = DEFAULT_LOCK_TIMEOUT,
    ) -> None:
        for name, value in (("dhash_max", dhash_max), ("phash_max", phash_max)):
            if not 0 <= value <= MAX_SUPPORTED_DISTANCE:
                raise DedupError(
                    f"{name}={value} is outside 0 to {MAX_SUPPORTED_DISTANCE}. "
                    f"The banded index holds {BAND_COUNT} bands, so it can only "
                    f"guarantee it finds every match up to {MAX_SUPPORTED_DISTANCE} "
                    "bits. A larger threshold would miss duplicates without saying so."
                )
        self.path = pathlib.Path(path)
        self.dhash_max = dhash_max
        self.phash_max = phash_max
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.db = sqlite3.connect(
            self.path, timeout=lock_timeout, isolation_level=None
        )
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA journal_mode=WAL")
        self.db.execute("PRAGMA synchronous=NORMAL")
        self.db.execute("PRAGMA foreign_keys=ON")
        self.db.execute(f"PRAGMA busy_timeout={int(lock_timeout * 1000)}")
        self._migrate()

    def _migrate(self) -> None:
        self.db.executescript("""
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS images (
                sha256   TEXT PRIMARY KEY,
                dhash    INTEGER NOT NULL,
                phash    INTEGER NOT NULL,
                source   TEXT NOT NULL,
                ref      TEXT NOT NULL,
                width    INTEGER NOT NULL,
                height   INTEGER NOT NULL,
                texture  REAL NOT NULL,
                added_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS bands (
                kind   TEXT NOT NULL,
                idx    INTEGER NOT NULL,
                band   INTEGER NOT NULL,
                sha256 TEXT NOT NULL REFERENCES images(sha256) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS bands_lookup ON bands(kind, idx, band);
            CREATE INDEX IF NOT EXISTS bands_owner ON bands(sha256);
            CREATE TABLE IF NOT EXISTS rejections (
                sha256       TEXT NOT NULL,
                source       TEXT NOT NULL,
                ref          TEXT NOT NULL,
                kind         TEXT NOT NULL,
                distance     INTEGER NOT NULL,
                matched      TEXT NOT NULL,
                rejected_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS rejections_matched ON rejections(matched);
        """)
        stored = dict(self.db.execute("SELECT key, value FROM meta").fetchall())
        expected = {
            "schema_version": str(SCHEMA_VERSION),
            "hash_bits": str(HASH_BITS),
            "band_count": str(BAND_COUNT),
        }
        if not stored:
            self.db.executemany(
                "INSERT INTO meta(key, value) VALUES (?, ?)", expected.items()
            )
            return
        for key, want in expected.items():
            have = stored.get(key)
            if have != want:
                raise DedupError(
                    f"{self.path} was built with {key}={have} and this code "
                    f"expects {key}={want}. The stored hashes are not comparable "
                    "with the ones this version computes. Rebuild the store or "
                    "use the matching version; do not mix them."
                )

    # SQLite integers are signed 64 bit and our hashes are unsigned 64 bit, so
    # the top bit would otherwise overflow the column type on insert.
    @staticmethod
    def _signed(value: int) -> int:
        return value - (1 << 64) if value >= (1 << 63) else value

    @staticmethod
    def _unsigned(value: int) -> int:
        return value + (1 << 64) if value < 0 else value

    def close(self) -> None:
        with contextlib.suppress(sqlite3.Error):
            self.db.close()

    def __enter__(self) -> DedupStore:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def count(self) -> int:
        return self.db.execute("SELECT COUNT(*) FROM images").fetchone()[0]

    def rejection_count(self) -> int:
        return self.db.execute("SELECT COUNT(*) FROM rejections").fetchone()[0]

    def find_match(self, fp: Fingerprint) -> Match | None:
        """The first collision found, exact before perceptual. None means new.

        Not a transaction in itself: `offer` wraps this and the insert together,
        which is the call that must be atomic.
        """
        row = self.db.execute(
            "SELECT sha256, source, ref FROM images WHERE sha256 = ?", (fp.sha256,)
        ).fetchone()
        if row:
            return Match(row["sha256"], row["source"], row["ref"], "exact", 0)

        for kind, value, limit in (
            ("d", fp.dhash, self.dhash_max),
            ("p", fp.phash, self.phash_max),
        ):
            best: Match | None = None
            bands = [(kind, i, fp.band(kind, i)) for i in range(BAND_COUNT)]
            clauses = " OR ".join(["(kind = ? AND idx = ? AND band = ?)"] * BAND_COUNT)
            column = "dhash" if kind == "d" else "phash"
            other = "phash" if kind == "d" else "dhash"
            other_value = fp.phash if kind == "d" else fp.dhash
            other_limit = self.phash_max if kind == "d" else self.dhash_max
            candidates = self.db.execute(
                f"""SELECT DISTINCT i.sha256, i.source, i.ref, i.texture,
                           i.{column} AS h, i.{other} AS other_h
                    FROM bands b JOIN images i ON i.sha256 = b.sha256
                    WHERE {clauses}""",
                [item for band in bands for item in band],
            )
            for cand in candidates:
                distance = hamming(value, self._unsigned(cand["h"]))
                if distance > limit:
                    continue
                # A thin picture hashes its own rounding noise, so one hash
                # agreeing proves nothing. Demand the other one as well.
                thin = (fp.texture < CORROBORATION_BELOW
                        or cand["texture"] < CORROBORATION_BELOW)
                if thin:
                    corroborating = hamming(
                        other_value, self._unsigned(cand["other_h"])
                    )
                    if corroborating > other_limit:
                        continue
                if best is None or distance < best.distance:
                    best = Match(
                        cand["sha256"], cand["source"], cand["ref"],
                        "dhash" if kind == "d" else "phash", distance,
                    )
                    if distance == 0:
                        break
            if best is not None:
                return best
        return None

    def offer(self, fp: Fingerprint, source: str, ref: str) -> Decision:
        """Admit this image if it is new, and record the rejection if it is not.

        The check and the insert share one IMMEDIATE transaction, so two workers
        offering the same picture at the same moment cannot both be told it is
        new. Idempotent: re-offering an image already held returns a rejection
        against itself rather than a second row.
        """
        if not source or not ref:
            raise DedupError("every image needs a source and a ref to be traceable")
        now = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
        self.db.execute("BEGIN IMMEDIATE")
        try:
            match = self.find_match(fp)
            if match is not None:
                self.db.execute(
                    """INSERT INTO rejections
                       (sha256, source, ref, kind, distance, matched, rejected_at)
                       VALUES (?, ?, ?, ?, ?, ?, ?)""",
                    (fp.sha256, source, ref, match.kind, match.distance,
                     match.sha256, now),
                )
                self.db.execute("COMMIT")
                return Decision(False, fp, match)

            self.db.execute(
                """INSERT INTO images
                   (sha256, dhash, phash, source, ref, width, height,
                    texture, added_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                (fp.sha256, self._signed(fp.dhash), self._signed(fp.phash),
                 source, ref, fp.width, fp.height, fp.texture, now),
            )
            self.db.executemany(
                "INSERT INTO bands(kind, idx, band, sha256) VALUES (?, ?, ?, ?)",
                [(kind, i, fp.band(kind, i), fp.sha256)
                 for kind in ("d", "p") for i in range(BAND_COUNT)],
            )
            self.db.execute("COMMIT")
            return Decision(True, fp)
        except Exception:
            with contextlib.suppress(sqlite3.Error):
                self.db.execute("ROLLBACK")
            raise

    def offer_file(self, path: pathlib.Path, source: str, ref: str) -> Decision:
        return self.offer(fingerprint_file(path), source, ref)

    def stats(self) -> dict[str, object]:
        by_source = {
            r["source"]: r["n"] for r in self.db.execute(
                "SELECT source, COUNT(*) AS n FROM images GROUP BY source ORDER BY n DESC"
            )
        }
        by_reason = {
            r["kind"]: r["n"] for r in self.db.execute(
                "SELECT kind, COUNT(*) AS n FROM rejections GROUP BY kind ORDER BY n DESC"
            )
        }
        return {
            "store": str(self.path),
            "accepted": self.count(),
            "rejected": self.rejection_count(),
            "accepted_by_source": by_source,
            "rejected_by_reason": by_reason,
            "dhash_max": self.dhash_max,
            "phash_max": self.phash_max,
        }

    def rejections(self):
        yield from self.db.execute(
            "SELECT * FROM rejections ORDER BY rejected_at, sha256"
        )


def _cmd_add(store: DedupStore, args: argparse.Namespace) -> int:
    root = pathlib.Path(args.directory)
    if not root.is_dir():
        print(f"not a directory: {root}", file=sys.stderr)
        return 2
    paths = sorted(p for p in root.rglob("*") if p.suffix.lower() in
                   (".png", ".jpg", ".jpeg", ".webp", ".bmp", ".tif", ".tiff"))
    if not paths:
        print(f"no images under {root}", file=sys.stderr)
        return 1

    accepted = rejected = failed = 0
    last_beat = time.monotonic()
    for n, path in enumerate(paths, 1):
        try:
            decision = store.offer_file(path, args.source, str(path.relative_to(root)))
        except DedupError as e:
            failed += 1
            print(f"  skipped: {e}", file=sys.stderr)
            continue
        if decision.accepted:
            accepted += 1
        else:
            rejected += 1
            if args.verbose:
                print(f"  reject {path.name}: {decision.match.describe()}")
        # Baseline Section 2.1: a long loop says it is alive at least every
        # 60 seconds, so an overnight run can be tailed rather than guessed at.
        if time.monotonic() - last_beat >= 30:
            print(f"  ... {n}/{len(paths)}: {accepted} kept, {rejected} duplicate",
                  flush=True)
            last_beat = time.monotonic()

    print(f"{root}: {accepted} accepted, {rejected} duplicate, {failed} unreadable")
    print(f"store now holds {store.count()} images")
    return 0 if accepted or not paths else 1


def _cmd_check(store: DedupStore, args: argparse.Namespace) -> int:
    try:
        fp = fingerprint_file(pathlib.Path(args.image))
    except DedupError as e:
        print(e, file=sys.stderr)
        return 2
    match = store.find_match(fp)
    print(f"sha256 {fp.sha256}")
    print(f"dhash  {fp.dhash:016x}")
    print(f"phash  {fp.phash:016x}")
    if match is None:
        print("new: no image in the store collides with this one")
        return 0
    print(f"duplicate: {match.describe()}")
    return 1


def _cmd_stats(store: DedupStore, _args: argparse.Namespace) -> int:
    import json
    print(json.dumps(store.stats(), indent=2))
    return 0


def _cmd_rejections(store: DedupStore, _args: argparse.Namespace) -> int:
    import json
    for row in store.rejections():
        print(json.dumps(dict(row)))
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--db", required=True, help="path to the dedup store")
    ap.add_argument("--dhash-max", type=int, default=DEFAULT_DHASH_MAX)
    ap.add_argument("--phash-max", type=int, default=DEFAULT_PHASH_MAX)
    sub = ap.add_subparsers(dest="command", required=True)

    p_add = sub.add_parser("add", help="offer every image in a directory")
    p_add.add_argument("directory")
    p_add.add_argument("--source", required=True, help="e.g. commons, unsplash")
    p_add.add_argument("--verbose", action="store_true")
    p_add.set_defaults(func=_cmd_add)

    p_check = sub.add_parser("check", help="test one image without admitting it")
    p_check.add_argument("image")
    p_check.set_defaults(func=_cmd_check)

    sub.add_parser("stats", help="what the store holds").set_defaults(func=_cmd_stats)
    sub.add_parser("rejections", help="every rejection as JSON lines").set_defaults(
        func=_cmd_rejections
    )

    args = ap.parse_args(argv)
    try:
        with DedupStore(args.db, args.dhash_max, args.phash_max) as store:
            return args.func(store, args)
    except DedupError as e:
        print(f"dedup: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

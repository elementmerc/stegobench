# Build a corpus

The published corpus is one tier of one run. Everything that made it is in the
repository, and the same commands make a different one.

## The shape of a run

```
    fetch_commons.py      covers + manifest.jsonl, one row per photograph
            │             with its licence, its credit line and its digest
            ▼
    build_jpeg_arms.py    the end-user tool arms, and the clean JPEG pool
            │             the DCT arms are then built from
            ▼
    build_adaptive_arms.py   the academic scheme arms, spatial and DCT
            │
            ▼
    pack_arms.py          WebDataset shards, each sample joined to its
            │             cover's licence
            ▼
    release_metadata.py   README, CITATION.cff, Croissant, datasheet, splits
```

`build_core_tier.py` runs the two builders as bounded, resumable jobs rather
than a shell loop, which matters more than it sounds: the full tier is roughly
six days of single-core CPU, and running it wide by hand is how this project
once put a sixteen-core machine at a load average of 131.

## Tiers are prefixes, not samples

A tier is the first *n* covers of one fixed ordering. Nano is 200, Lite is
1,000, Core is 10,000.

```
    ordering:  [ 0 1 2 3 4 5 6 7 8 9 ... ]
    Nano       └─────┘
    Lite       └───────────┘
    Core       └──────────────────────┘
```

This is load-bearing. Both builders originally took a seeded random sample,
which is reproducible and still wrong: `sample(pool, 200)` is not the first 200
of `sample(pool, 1000)`. Somebody trains on Lite, evaluates on Core, and
silently tests on images they trained on. Every number they publish is inflated
by an amount nobody can recover afterwards.

CI checks the nesting at all three sizes on all three operating systems, and a
tier larger than the corpus is refused rather than truncated.

## Building a small one

```sh
python3 generators/fetch_commons.py --out covers/ --count 200 \
    --dedup-db dedup.sqlite3 --licences permissive

python3 generators/build_adaptive_arms.py --covers covers/ --out arms/adaptive \
    --count 200 --schemes wow --rates 0.4

python3 generators/pack_arms.py --arms arms/ \
    --covers-manifest covers/manifest.jsonl --out packed/
```

`tools/ci/tier_smoke.py` does exactly this against synthetic covers, and
asserts at the end that every packed sample carries a cover licence and names
the cover it came from.

## The licence travels with the pixels

A stego image is a derivative of the photograph it was made from, and 54% of
the covers require attribution. So every sample JSON carries the cover's
licence, artist, credit line and description URL under `cover_licence`. A
reader who downloads one arm and never opens the cover tier still has
everything the licence asks of them.

The packer refuses to ship a sample whose cover it cannot name. That is not a
warning; the sample is left out and the run reports a non-zero exit.

The spatial arms name their cover directly. The DCT arms name it through the
clean JPEG pool, by the positional filename that pool used, and the packer
resolves that second hop from the manifest that recorded it. Each sample says
which route was taken, in a `licence_join` field.

## Determinism

Every generator is seeded from a per-cover, per-rate function of one run seed,
so two runs produce the same pixels. Shards are written with fixed member
metadata, so the same corpus packs to the same bytes on any machine and on any
day; a shard whose digest moves with the clock cannot be checked against a
published one.

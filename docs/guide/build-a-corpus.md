# Build a corpus

The published corpus is one tier of one run. The same commands make a different
one.

## The shape of a run

```
    fetch_commons.py         covers + manifest.jsonl, one row per photograph
            │                with its licence, credit line and digest
            ▼
    build_jpeg_arms.py       the end-user tool arms, and the clean JPEG pool
            │                the DCT arms are built from
            ▼
    build_adaptive_arms.py   the academic scheme arms, spatial and DCT
            │
            ▼
    pack_arms.py             WebDataset shards, each sample joined to its
            │                cover's licence
            ▼
    release_metadata.py      README, CITATION.cff, Croissant, datasheet, splits
```

`build_core_tier.py` runs the builders as bounded, resumable jobs. Use it for
anything larger than a few hundred covers: the full tier is roughly six days of
single-core CPU, and each worker is one busy core and about half a gigabyte.

## Building a small one

```sh
python3 generators/fetch_commons.py --out covers/ --count 200 \
    --dedup-db dedup.sqlite3 --licences permissive

python3 generators/build_adaptive_arms.py --covers covers/ --out arms/adaptive \
    --count 200 --schemes wow --rates 0.4

python3 generators/pack_arms.py --arms arms/ \
    --covers-manifest covers/manifest.jsonl --out packed/
```

`tools/ci/tier_smoke.py` runs exactly this against synthetic covers and asserts
every packed sample carries a cover licence and names its cover.

## Tiers are prefixes

A tier is the first *n* covers of one fixed ordering, so tiers nest:

```
    ordering:  [ 0 1 2 3 4 5 6 7 8 9 ... ]
    Nano       └─────┘
    Lite       └───────────┘
    Core       └──────────────────────┘
```

A seeded random sample is reproducible and still wrong here, because
`sample(pool, 200)` is not the first 200 of `sample(pool, 1000)`. A tier larger
than the corpus is refused rather than truncated.

## The licence travels with the pixels

Every sample carries its cover's licence, artist, credit line and source URL.
The packer refuses to ship a sample whose cover it cannot name: the sample is
left out and the run exits non-zero.

Spatial arms name their cover directly; DCT arms name it through the clean JPEG
pool, and the packer resolves that from the manifest that recorded it. Each
sample says which route it took in `licence_join`.

## Determinism

| | |
|---|---|
| Pixels | Seeded per cover and per rate, so two runs agree |
| Shards | Fixed member metadata, so the same corpus packs to the same bytes on any machine on any day |

A shard whose digest moves with the clock cannot be checked against a published
one.

# Build a corpus

The published corpus is one tier of one run. The same commands make a different
one.

## The shape of a run

```
    fetch-commons          covers + manifest.jsonl, one row per photograph
            │              with its licence, credit line and digest
            ▼
    manifest-repair        the tier order, assigned once over the finished
            │              cover set. Nothing below this line runs without it
            ▼
    build-jpeg-arms        the end-user tool arms, and the clean JPEG pool
            │              the DCT arms are built from
            ▼
    build-adaptive-arms    the academic scheme arms, spatial and DCT
            │
            ▼
    pack-arms              WebDataset shards, each sample joined to its
            │              cover's licence
            ▼
    release-metadata       README, CITATION.cff, Croissant, datasheet, splits
```

**`manifest-repair` is a step, not a repair.** A tier is a prefix of one
ordering over the whole corpus, and no single fetch can know that ordering, so
`fetch-commons` deliberately does not write `tier_order`. Skip this step and
every builder below it refuses, which is the right answer and an easy one to
read as a broken install.

`build-core-tier` runs the builders as bounded, resumable jobs. Use it for
anything larger than a few hundred covers: the Core tier is about 160 CPU-hours
of embedding, which is roughly 16 hours at the default 10 workers or 54 hours
at 3. Each worker is one busy core and about half a gigabyte, so memory decides
the number rather than core count. Run `pentimento build-core-tier --dry-run`
for your own tier and worker count, and budget the cover fetch separately: it's
the larger of the two.

## Building a small one

Four commands, in this order, with the same `--count` everywhere it appears. A
tier size that disagrees between two of them is the most common way this goes
wrong: `pack-arms` defaults to 10,000, which is right for the published corpus
and refuses anything smaller that forgets to say so.

```sh
pentimento fetch-commons --out covers/ --count 200 \
    --dedup-db dedup.sqlite3 --licences permissive

pentimento manifest-repair covers/manifest.jsonl

pentimento build-adaptive-arms --covers covers/ --out arms/adaptive \
    --count 200 --schemes wow --rates 0.4

pentimento pack-arms --arms arms/ \
    --covers-manifest covers/manifest.jsonl --out packed/ --count 200
```

The fetch is the slow one: Commons is rate limited to one request per second
and roughly four and a half candidates are drawn per cover kept, so 200 covers
is about twenty minutes. It prints a count and a rate every thirty seconds, and
re-running the same command resumes rather than starting again.

`tools/ci/tier_smoke.py` runs the build and the pack against synthetic covers,
writing their manifest with the tier order already in it rather than fetching
and repairing, and asserts every packed sample carries a cover licence and
names its cover.

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

Every sample carries its cover's licence, artist, credit line and source URL,
because a stego image is a derivative work and inherits its cover's terms. The
packer refuses to ship a sample whose cover it cannot name: the sample is left
out and the run exits non-zero.

Spatial arms name their cover directly; DCT arms name it through the clean JPEG
pool, and the packer resolves that from the manifest that recorded it. Each
sample says which route it took in `licence_join`.

A tool that rewrites the whole file gets a clean half written by that same
tool, so the pair differs in the payload and nothing else. Each sample records
which pairing it got.

## The split travels with the pixels too

Every sample also carries `split`, the side of the train and test boundary its
cover belongs to. The boundary is a property of the cover, so a cover and every
stego image made from it land on the same side, and a detector is never
evaluated on an image it was trained on.

That field is what makes the rule followable from a shard alone. The split is
decided once, in the cover manifest, and a downloader doesn't have the cover
manifest: without the field in the record, the rule would be something you
could read about and not check.

The packer refuses to ship a sample whose cover has no side recorded, and says
how many it left out and what to run to fix it. A sample with no side is one a
reader will put on both.

## Determinism

| | |
|---|---|
| Pixels | Seeded per cover and per rate, so two runs agree |
| Shards | Fixed member metadata, so the same corpus packs to the same bytes on any machine on any day |

A shard whose digest moves with the clock cannot be checked against a published
one.

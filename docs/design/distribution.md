# How Pentimento is packaged and published

Two decisions live here: what sizes the corpus ships in, and where it is hosted.
Both were settled on 2026-09-16.

## What this corpus is, stated before anything else

**Pentimento is a JPEG-decompressed spatial corpus. It is not a never-compressed
one, and it is not comparable to BOSSbase.**

Every one of the 10,000 covers was a JPEG before we cropped it. The manifest has
always said so on every row (`original_mime: image/jpeg`, `pristine: false`,
10,000 of 10,000), and an earlier version of this document claimed BOSSbase
comparability anyway. That claim was false and it is gone.

The consequence is concrete rather than theoretical. The 8x8 JPEG block lattice
survives decompression and is recoverable from the pixels: measured across a
random sample, the block phase can be predicted exactly from the `crop_box` this
manifest publishes, in 7 cases out of 8. The least significant bit plane is
therefore a function of quantised DCT coefficients rather than of sensor noise,
and on the two thirds of covers with chroma subsampling, two of three channels
are upsampler output.

**What that means for a user.** Thresholds calibrated here will not transfer
unchanged to never-compressed covers. This project has already measured that
exact domain shift once, when thresholds set on never-JPEG corpora leaked around
22% false positives against JPEG-decompressed ALASKA2. A corpus of 10,000 more
JPEG-decompressed covers does not fix that; it is the other side of it.

**Why it is still worth publishing.** Almost all real imagery is JPEG at some
point, so this is the realistic regime rather than the laboratory one, and no
corpus of this size offers it with verified per-file licensing and this
diversity. It is a different contribution from BOSSbase, not a replacement for
it.

### A never-compressed arm has no source, and that is measured

The obvious fix is to add an arm of never-compressed covers. The fetcher already
supports it: `suitable()` accepts TIFF and PNG and `provenance.pristine()`
identifies them. So the question is supply, and on 2026-09-18 it was measured
rather than assumed, with `generators/probe_pristine_supply.py`.

**Commons.** Six targeted searches, including ones naming camera makers
directly, over 5,084 and 8,416 TIFF hits respectively:

| Query | Hits | TIFFs checked | With camera EXIF |
|---|---|---|---|
| `filemime:tiff Nikon` | 5,084 | 40 | **0** |
| `filemime:tiff Canon EOS` | 8,416 | 40 | **0** |
| `filemime:tiff photograph landscape` | 325,809 | 40 | **0** |
| `filemime:tiff portrait photo` | 10,429 | 40 | **0** |
| `filemime:tiff DSLR` | 7 | 7 | **0** |
| `filemime:tiff camera raw converted` | 53 | 40 | **0** |

**207 TIFFs, not one with a camera Make.** The positive control, the identical
extraction run over JPEGs, found 32 of 40 (80%), so the method works and the
zero is the answer rather than a bug. Commons TIFFs are manuscript scans, maps
and artwork reproductions: cameras write JPEG or RAW, and Commons hosts neither
as a photographic original in any quantity.

**Everywhere else.** `cover-source-licensing.md` already closes the academic
lineage: ALASKA2 is no-derivatives, BOSSbase has no licence anyone can produce,
and Dresden, RAISE, BOWS2 and DIV2K are non-commercial. There is no corpus of
never-compressed camera originals that we may lawfully republish.

**So the arm is not deferred for want of effort.** The only remaining route is
photographing RAW ourselves, which is what the BOSS organisers did with seven
cameras, and which buys unambiguous rights at a small volume. Until then this
corpus is JPEG-decompressed and says so at the top.

## Tiers, and why the nesting is the load-bearing part

Almost nobody needs the whole corpus. A developer wiring a detector into a
product wants enough images to know their code runs and their thresholds are
sane; a researcher training a network wants everything. Shipping only the second
means the first downloads a terabyte to run a smoke test, and most of them
simply will not.

| Tier | Covers | Size (covers only / with stego arms) | Intended reader |
|---|---|---|---|
| **Nano** | 200 | 66 MB / 1.0 GB | Continuous integration and smoke tests. Downloads in seconds |
| **Lite** | 1,000 | 327 MB / 4.8 GB | A developer checking their integration against real data |
| **Core** | 10,000 | 3.3 GB / 48 GB | **Version 1.** The publishable corpus |

A larger tier than Core is not built and is deliberately not sized here: a
projected figure next to three measured ones invites a reader to treat it as
equally real.

### The requirement that makes tiers safe

**The tiers are strictly nested: Nano is a subset of Lite, which is a subset of
Core. There is exactly one train and test split, defined once over the covers
that exist and inherited downward.**

This is not tidiness. Without it, the obvious mistake is invisible and fatal:
somebody trains on Lite, evaluates on Core, and unknowingly tests on images
they trained on, because the two tiers were sampled independently and overlap.
Every number they publish is then inflated by an amount nobody can recover after
the fact.

That is the same defect the deduplication store exists to prevent inside the
corpus, reintroduced at the distribution layer where dedup cannot see it. It is
a documented flaw in MIRFLICKR and it would be entirely our fault here.

So, concretely:

- The split assignment is a property of the **cover**, decided once, recorded in
  the manifest, and never recomputed per tier.
- A tier is a prefix of a single deterministic ordering of covers, not a fresh
  sample. Nano is the first 200 of that order, Lite the first 1,000, and so on.
- Every tier carries the same arm structure, so code written against Nano runs
  unchanged against a larger tier and only the numbers move.
- The manifest of a smaller tier is a strict subset of the larger one's, line for
  line, so a checksum comparison proves the nesting rather than asserting it.

### How it is implemented, and what each field guarantees

Both fields were described here before they existed, so the guarantee above was
prose with nothing behind it. Both are now written by
`generators/manifest_repair.py`, and the fetcher emits `split` directly.

| Field | Derived from | What it survives |
|---|---|---|
| `split` | SHA-256 of the cover's own Commons content hash plus a fixed salt | The corpus growing, being reordered, or being rebuilt. No existing assignment ever moves |
| `tier_order` | Assigned once over the covers present, recorded, and appended to thereafter | New covers arriving. They take positions after the existing ones rather than interleaving |
| `split_salt` | Recorded on every row | Somebody changing the salt later without noticing what it invalidates |

The ordering is append-stable rather than a hash sort, and that is the whole
point. A hash sort reshuffles every position each time a cover is added, so
Nano's first 200 would be a different 200 in every release, and a prefix that
stops being a prefix is worse than having no tiers at all.

Current assignment over the 10,000 covers: 8,032 train and 1,968 test, with
Nano at 16.5% test and Lite at 19.0%. Those fractions differ slightly by tier
because the split is a property of the cover rather than of the tier, which is
exactly the property being bought.

Nobody else tiers a steganalysis corpus. Done properly it is a differentiator;
done carelessly it is a way to poison every result built on the dataset.

## Packing: not half a million loose files

Version 1 is roughly 520,000 images. HuggingFace publishes its limits plainly:
**under 10,000 files per folder, and under 100,000 files per repository
recommended**. Shipping loose PNGs breaks the platform we most want to be on,
and it is slow to download everywhere else too.

The corpus therefore ships as **sharded archives** (WebDataset tar shards, with
Parquet for the manifests). Both are formats HuggingFace explicitly recommends
and its dataset viewer understands.

This also fixes a duplication problem in the generators. `embed_adaptive.py`
writes a clean copy of the cover beside every stego image, and at five schemes
across six payload rates that is **thirty identical copies of every cover**.
Measured: version 1 as written is 205 GB, against roughly 107 GB with each cover
stored once. The packing format and the deduplication are the same change, which
is why they are being done together rather than in sequence.

## Hosting

Targets, and the rule that picks them for version 1: **free, effectively
unlimited, and needing no application or approval**, because an approval queue
is not a plan you can schedule around.

| Destination | Version 1 | Capacity | Why |
|---|---|---|---|
| **Internet Archive** | **yes** | unlimited, free | No approval, no size ceiling, permanent, and it has outlived most of the hosts in `cover-source-licensing.md` |
| **Academic Torrents** | **yes** | unlimited, free | Built precisely for this; costs nothing to seed and scales with the corpus |
| **Kaggle** | **yes** | about 200 GB per dataset | No approval, and it is where practitioners actually look. Core fits comfortably |
| **Zenodo** | later | 50 GB per record by default, more on request | The DOI and the tombstoned withdrawal make it the right home for a citable curated tier, but the size request is an approval step |
| **AWS Open Data Registry** | later | free hosting | How iNaturalist itself is hosted. Requires an application |
| **HuggingFace** | later | best-effort free, grants available | The free public tier is "best-effort", not unlimited. Storage grants exist for high-impact open work through `datasets@huggingface.co`, and that is worth applying for **once there is a corpus and download numbers to point at**, not before |

Version 1 goes only to the free, unlimited, no-approval destinations. The
others are targets, not blockers.

### One release process, not five upload scripts

Every destination is fed from the same per-file licence manifest, so a
platform's licence field cannot drift from what the corpus actually contains.
That drift is exactly what produced Dresden-as-CC0 and UCID-as-MIT in the
mirrors surveyed in `cover-source-licensing.md`, and it is the single most
likely way this corpus becomes the thing it was built to correct.

Publishing to one destination and then to the others must be one command with
one source of truth. A human copying a licence string into a web form is the
failure mode being designed out.

## Reproducible, or re-derivable? Only one of these is true

The corpus is **re-derivable, not reproducible**, and the distinction is not
pedantry.

Re-running `fetch_commons.py` will not rebuild this corpus. Acquisition draws
from Commons' live random generator, no acquisition seed is recorded, and both
the diversity caps and the deduplication store depend on arrival order. Two runs
with identical arguments produce two different corpora.

What the manifest does guarantee is exact re-derivation. Every row carries
`pageid`, `commons_sha1`, `crop_box` and `sha256`, all unique across the corpus,
which is enough for anyone to fetch the same source file from Commons, take the
same crop, and verify they got the same bytes we did.

So the honest claim is: **you cannot regenerate this corpus, and you can verify
every image in it.** Earlier drafts of this document said "reproducible", which
promised the first.

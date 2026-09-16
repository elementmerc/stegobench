# Pentimento

The corpus this repository builds.

A *pentimento* is an earlier image or brushstroke that becomes visible through
the paint layer as it ages: the hidden thing, surfacing. That is steganalysis in
one word, and it is the only name considered here that describes detection rather
than hiding.

**The name is deliberate in a second way.** The obvious name was taken. "StegBench"
now refers to an August 2026 MDPI dataset paper claiming 525,000 images and
shipping none, a 2021 MIT-licensed tool at `DAI-Lab/stegbench`, and a gated LLM
covert-channel corpus. `stegobench` is one letter from the first of those, and a
reviewer searching the name would find a same-year dataset paper making adjacent
claims. So:

- **stegobench** stays the *toolkit*: the maintained multi-tool Docker image and
  the generators, which is what the portfolio entry scoped on 2026-05-22. A name
  collision matters far less for a container than for a dataset citation.
- **Pentimento** is the *corpus*.

## What it is for

One corpus spanning **real end-user tools and academic content-adaptive schemes
together**, across spatial and JPEG domains, at camera diversity, down to very low
payload, as **separate labelled arms rather than a blend**.

Nobody holds that position. REVEAL (Netherlands Forensic Institute, 2025) covers
51 real tools superbly and excludes adaptive schemes on purpose. Everything with
good adaptive coverage is JPEG-only, derived from BOSSbase, and licensed so
restrictively it cannot be redistributed. See `reveal.md` and
`cover-source-licensing.md`.

### The two claims, in the order they should be made

**1. Per-source arms, not a blend.** Ker et al. warned in 2013: *"It is
fallacious to try to train on a large heterogeneous data set as somehow
'representative' of mixed sources, because it guarantees a mismatch and may still
be an unrepresentative mixture."* Sedighi and Fridrich later measured the
consequence: the *ranking of embedding schemes inverts* when the cover source
changes, with WOW the least secure scheme on BOSSbase and the most secure on
decompressed JPEGs. No published corpus ships stratified arms that let a user
measure that rather than suffer it. This one does, and it is the headline, not
the scale.

**2. Byte-exact rebuild.** The BOSSbase that deep-learning steganalysis actually
uses is a *recipe*, not a file: resize with Matlab `imresize` at defaults,
recompress at quality 75 or 95, split 14,000/1,000/5,000. Nobody ships checksums,
and `imresize` defaults are a version dependency no paper records. Tabares-Soto
et al. measured a nine-point accuracy swing from the data split alone, same
network, same algorithm. Every generator here is seeded and writes a manifest
with a sha256 per file and the count of samples actually changed.

## Version 1 target

**10,000 covers.** Exactly BOSSbase's count, deliberately, so every number is
directly comparable to fifteen years of published results.

| Arm | Methods | Rates | Stego images |
|---|---|---|---|
| Adaptive spatial | HUGO, WOW, S-UNIWARD, HILL, MiPOD | 0.4, 0.2, 0.1, 0.05, 0.01, 0.005 bpp | 300,000 |
| LSB | replacement, matching | same six | 120,000 |
| Real tools | steghide, outguess, openstego | three rates | ~90,000 |
| Structural | data appended after end-of-image | fixed | 10,000 |
| **Total** | | | **~520,000** |

Every pair is matched: cover and stego are the same picture through the same code
path, differing only in the embedded bits.

## Cover sources

Only sources whose terms plainly permit republishing a derived work. From
`cover-source-licensing.md`:

**In:** Wikimedia Commons (derivative publication is an entry requirement),
Unsplash under its ordinary licence, CLIC, Open Images.

**Out:** ALASKA (no-derivatives), BOSSbase (never permissively licensed), Dresden,
RAISE and BOWS2 (non-commercial only, which collides with Stegcore's commercial
tier), and a long tail granting nothing at all.

**Pexels is disputed** and the 200 covers already fetched are affected: its
Licence permits modification while its Terms of Service ban bulk automated
collection. Resolve or replace before publishing.

## Deduplication is a correctness requirement, not hygiene

No two images in the corpus may be the same, and exact hashing is not enough.
Three failure modes:

1. **The same photo on two services.** Heavily reposted images appear on both
   Unsplash and Commons.
2. **Near-duplicates within one service.** Burst frames, crops, recolours.
3. **Cross-arm leakage.** The same cover in both train and test silently inflates
   every result built on the corpus. This is a documented defect in MIRFLICKR and
   it is invisible unless you look for it.

So: sha256 as the cheap first pass, then perceptual hashing (dHash and pHash) with
a persistent store across every source and every session. A candidate within a
small Hamming distance of anything already held is rejected, **and the rejection
is recorded in the manifest**, so the corpus can show what it excluded and why.

## Resource budget on atlas

Measured 2026-09-16, not estimated.

| | |
|---|---|
| Throughput, one core | 0.22 s/image (HILL) to 1.25 s/image (MiPOD); mean 0.82 |
| Peak resident per worker | 246 MB, or 546 MB for MiPOD, at 512px |
| atlas | 16 cores, 28 GB, and **`llama-server` holds close to 20 GB of it** |

**Three workers, not more.** CPU is idle (load 0.15) and memory is the binding
constraint: the local model must not be pushed into swap, which is the exact
failure holst's resource ledger exists to prevent. Three workers peak around
1.6 GB, roughly 15% of what is free.

At three workers the whole v1 corpus is about **40 hours of work**, which fits a
four-day window with deliberate slack. The long pole is cover acquisition and
deduplication, not embedding: Commons at one polite request per second is the
floor, and dedup has to run inline rather than as a later pass.

## Publication

The dataset goes to HuggingFace, Kaggle and Zenodo from **one release process**,
not three upload scripts, with the per-file licence manifest feeding each
platform's licence field so they cannot drift apart. That drift is exactly what
produced Dresden-as-CC0 and UCID-as-MIT, and it is the single most likely way
this corpus becomes the thing it was built to correct.

Zenodo matters beyond reach: it issues a DOI and supports tombstoned withdrawal,
which is the only honest failure mode if anything ever has to come down.

## Calibrating Stegcore against REVEAL first

Before Pentimento exists, REVEAL is the best available calibration target and
costs nothing licensing-wise, because running a detector over a corpus is use
rather than redistribution. It has over 50 real tools, payload rates four orders
of magnitude below where the field's grids stop, and over 200 image sizes against
Stegcore's 512x512-only calibration history. Details in `reveal.md`.

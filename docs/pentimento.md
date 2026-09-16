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

### How Commons is sampled, and why it is not the obvious way

Measured on 2026-09-16 against the live API, because every number here changes
what the overnight fetch collects.

The first ten categories this project used were featured and quality picture
sets. That reaches an aesthetically selected slice: competent photographers,
good light, and a camera population skewed towards expensive bodies. A corpus
built from it measures detection on prize-winning photographs.

Uniform random sampling of the file namespace has the opposite bias, and it is
worse. Commons' public domain holdings are dominated by bulk archive
digitisation, so random draws return scanned books, engravings, maps and
diagrams. In a 25 cover test run the titles included an undergraduate course
catalogue, a 1924 Polish physics textbook, a plate of French royal coinage and a
sixteenth century treatise on fishes. Scanned text is a legitimate cover class
with its own statistics, sharply bimodal histograms and large flat regions, and
it is not a photograph. It gets its own arm or it gets excluded; it does not get
mixed in silently.

**A camera make in the EXIF is the cheapest available proof a camera made the
file**, and it doubles as the acquisition-diversity axis. Across 237 random
draws that were JPEG or PNG and at least 512px on each side:

| | Permissive | Share-alike or other |
|---|---|---|
| **Carries camera EXIF** | 52 | 79 |
| **No EXIF** (scans, diagrams) | 78 | 28 |

The two filters pull against each other, and that is the whole finding. The
public domain bulk *is* the scans. Photographers who upload to Commons
overwhelmingly choose CC BY-SA: 56.5% of the photographs in that sample. So

- photograph **and** permissive: **21.9%** per draw
- photograph, any licence: **55.3%** per draw

Excluding share-alike costs roughly 60% of every photograph Commons holds.

**Ruled 2026-09-16: permissive only.** CC0, public domain and plain CC BY. No
share-alike, deliberately, so that anyone can use this corpus without first
working out what obligation it puts them under. The operator's reasoning was
adoption rather than scale, and the arithmetic supports it: Commons holds
147,766,476 files, which puts the permissive photograph pool near 25 million, so
the 10,000 cover target is 0.04% of what is available and even a 100,000 cover
corpus is under 0.4%. The pool never binds. Share-alike would have bought
throughput we do not need at the price of an obligation every downstream user
would have to reason about.

The diversity on offer is the reason to bother: **107 distinct camera models
across 131 photographs**, with Apple, Xiaomi, Samsung and Google appearing
alongside Canon, Nikon, Sony, Pentax and Olympus. Phone cameras are not a
contaminant here, they are what a deployed detector actually meets.

One operational note: the API answered HTTP 429 at 0.4 second spacing. A second
between calls, with the backoff the fetcher already carries, is the floor.

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

### Redundancy is a fourth failure mode, and hashing cannot touch it

There is a fourth case the three above do not cover, and it was found by looking
for it. A 20 cover test run returned four frames of `ISS0xx-E-xxxxx - View of
Earth`, every one shot on the same Nikon D4 aboard the space station, because
NASA has uploaded tens of thousands of them and uniform random sampling weights
files rather than photographers.

Those frames are not duplicates. They are different continents under different
clouds, and deduplication is right to admit each one. The corpus still ends up
with a visible share of one camera pointed at one subject.

Measured on 2026-09-16, pHash distances across 7 such frames and 20 unrelated
Commons photographs:

| | min | median |
|---|---|---|
| Within the ISS set | 24 | 30 |
| Within unrelated photographs | 24 | 32 |
| ISS against unrelated | 22 | 32 |

**The distributions sit on top of each other.** Two photographs of Earth from
the station are no more alike, to a perceptual hash, than two photographs of
unrelated things, so no threshold separates them. Raising the radius to reach
that far fails independently: chance collisions run at 1 in 26 billion at
distance 7 and 1 in 82,086 at distance 15, which at 25 million images is 305
false collisions for every candidate offered.

Redundancy is therefore handled where its evidence actually lives, in the
acquisition metadata. Commons returns the uploader and the camera in the same
response the fetcher already reads, so `DiversityCaps` limits how many covers
one uploader or one camera body may contribute, checked before the download so a
cover we will not keep costs no bandwidth. The caps are recorded in the manifest
and the tally resumes from it, since a cap that resets on restart is a cap that
doubles overnight.

### The same measurement found a false rejection

The closest pair in that whole set was not two ISS frames. It was an ISS frame
and an unrelated photograph, 6 bits apart on dHash, exactly the threshold, while
pHash put them 28 apart. Both had almost no texture, 1.00 and 1.43 grey levels,
which is above the cover floor and inside its advisory band.

dHash records which of each adjacent pair of pixels is brighter, so where there
are no gradients those comparisons are decided by rounding and two unrelated
thin pictures agree about as often as two coins do. pHash fails the same way for
the same reason. A picture that thin now needs both hashes to agree before it is
refused, which two independent noise sources will not do, and ordinary
photographs are unaffected: a resave and a brightness shift are still caught at 0
bits, an 8 pixel crop with resize at 2.

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

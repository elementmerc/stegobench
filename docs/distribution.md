# How Pentimento is packaged and published

Two decisions live here: what sizes the corpus ships in, and where it is hosted.
Both were settled on 2026-09-16.

## Tiers, and why the nesting is the load-bearing part

Almost nobody needs the whole corpus. A developer wiring a detector into a
product wants enough images to know their code runs and their thresholds are
sane; a researcher training a network wants everything. Shipping only the second
means the first downloads a terabyte to run a smoke test, and most of them
simply will not.

| Tier | Covers | Approximate size | Intended reader |
|---|---|---|---|
| **Nano** | 200 | ~1 GB | Continuous integration and smoke tests. Downloads in seconds |
| **Lite** | 1,000 | ~20 GB | A developer checking their integration against real data |
| **Core** | 10,000 | ~107 GB | **Version 1.** The publishable corpus, comparable to BOSSbase by construction |
| **Full** | 100,000 | ~1.1 TB | Training. The tier that needs sponsored storage |

### The requirement that makes tiers safe

**The tiers are strictly nested: Nano is a subset of Lite, which is a subset of
Core, which is a subset of Full. There is exactly one train and test split, and
it is defined at Full and inherited downward.**

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
  unchanged against Full and only the numbers move.
- The manifest of a smaller tier is a strict subset of the larger one's, line for
  line, so a checksum comparison proves the nesting rather than asserting it.

Nobody else tiers a steganalysis corpus. Done properly it is a differentiator;
done carelessly it is a way to poison every result built on the dataset.

## Packing: not half a million loose files

Version 1 is roughly 520,000 images and the full tier would be 5,200,000.
HuggingFace publishes its limits plainly: **under 10,000 files per folder, and
under 100,000 files per repository recommended**. Shipping loose PNGs breaks the
platform we most want to be on, and it is slow to download everywhere else too.

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
unlimited, and needing no application or approval**, because the report is owed
at the end of September and an approval queue is not a plan.

| Destination | Version 1 | Capacity | Why |
|---|---|---|---|
| **Internet Archive** | **yes** | unlimited, free | No approval, no size ceiling, permanent, and it has outlived most of the hosts in `cover-source-licensing.md` |
| **Academic Torrents** | **yes** | unlimited, free | Built precisely for this; costs nothing to seed and scales to the full tier later |
| **Kaggle** | **yes** | about 200 GB per dataset | No approval, and it is where practitioners actually look. Core fits; Full will not |
| **Zenodo** | later | 50 GB per record by default, more on request | The DOI and the tombstoned withdrawal make it the right home for a citable curated tier, but the size request is an approval step |
| **AWS Open Data Registry** | later | free hosting | How iNaturalist itself is hosted, and the natural home for the full tier. Requires an application |
| **HuggingFace** | later | best-effort free, grants available | The free public tier is "best-effort", not unlimited. Storage grants exist for high-impact open work through `datasets@huggingface.co`, and that is worth applying for **once there is a corpus and download numbers to point at**, not before |

The operator's instruction on this was explicit: version 1 goes only to the free,
unlimited, no-approval destinations. The others are targets, not blockers.

### One release process, not five upload scripts

Every destination is fed from the same per-file licence manifest, so a
platform's licence field cannot drift from what the corpus actually contains.
That drift is exactly what produced Dresden-as-CC0 and UCID-as-MIT in the
mirrors surveyed in `cover-source-licensing.md`, and it is the single most
likely way this corpus becomes the thing it was built to correct.

Publishing to one destination and then to the others must be one command with
one source of truth. A human copying a licence string into a web form is the
failure mode being designed out.

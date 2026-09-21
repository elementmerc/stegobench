# Corpus generators

Forty-four programs that fetch covers, build arms, check licences, pack a
release and verify it before it leaves the machine.

This file used to describe three of them and claim to describe all of them,
which is how a directory grows to forty-four without anybody noticing. The
table below is hand-written, so `test_readme.py` fails when a module is added
without a row: the claim that it goes stale loudly is only true if something
checks, and the first version of this file made that claim while two modules
were already missing from it.

## The pairing rule, which is the whole method

Every generator writes the cover and the stego image from the same source
array, through the same code path, so the two differ **only** in the embedded
payload. Where a resize is involved the cover is resized first and the payload
embedded afterwards, so resampling artefacts are identical in both halves.

This is the reliability of the method rather than a detail. If clean and stego
differ in any other way, a resave, a different quantisation table, stripped
metadata, then a detector can score well by recognising that difference and the
result says nothing about steganography.

**Three measurements have been lost to this class of error**, and the third is
the one worth remembering because the first two fixes did not prevent it:

1. outguess re-encoded at quality 75 regardless of input, so the halves came
   from different quality settings. Fix: same writer for both halves.
2. Two different encoders wrote the two halves. Fix: same writer, again.
3. Same writer, **different number of passes**. `jpeglib` prepends a JFIF APP0
   segment on every write, so a clean half written once carried two and a stego
   half written twice carried three. 80,000 images separable by reading the
   header. "Same writer" was checked and was true; nothing in that phrase
   contains "same number of passes".

So the rule is enforced structurally now rather than restated: `pack_arms.py`
parses both halves down to the scan and refuses any pair whose containers
differ. See `../docs/design/matched-pairs.md`.

## Reading the table

`CLI` means the module has a `__main__` and is meant to be run.
`lib` means other modules import it and running it directly does nothing
useful.

### Covers: acquisition, quality, provenance

| Module | | What it does |
|---|---|---|
| `fetch_commons.py` | CLI | Fetch cover images from Wikimedia Commons, with per-file licence provenance |
| `fetch_pexels.py` | CLI | Fetch royalty-free cover images from Pexels, with provenance |
| `probe_pristine_supply.py` | CLI | Does Commons hold never-compressed camera originals, and how many? |
| `cover_quality.py` | CLI | Whether a candidate crop is a usable steganographic cover at all |
| `dedup.py` | CLI | The deduplication store: one image may enter the corpus once, and only once |
| `provenance.py` | lib | What a cover carries with it: how it was compressed, and where it was cut from |
| `refit_covers.py` | CLI | Rebuild already-fetched covers through the current pipeline |
| `audit_covers.py` | CLI | Does the cover set actually meet the standard it claims? Check, and repair |
| `manifest_repair.py` | CLI | Add the fields the manifest promised and did not carry, and fix one that lied |

### Licences, which are why this corpus exists

| Module | | What it does |
|---|---|---|
| `audit_pd_licences.py` | CLI | Find out WHY each "Public domain" cover is public domain, and record it |
| `select_unpublishable.py` | CLI | Name the covers this corpus has no business redistributing, and say why |
| `backfill_covers.py` | CLI | Replace the covers we cannot publish, in place, keeping the tiers nested |

### Arms: the embedders and the sweeps

| Module | | What it does |
|---|---|---|
| `embedders.py` | lib | The real end-user tools, behind one interface |
| `tools.py` | CLI | The eight embedders, each with the quirk that makes it different |
| `payloads.py` | lib | Payload bytes that depend on which image they are for, and on nothing else |
| `embed_adaptive.py` | CLI | Content-adaptive spatial embedding: HUGO, WOW, S-UNIWARD, HILL, MiPOD |
| `build_adaptive_arms.py` | CLI | The adaptive arms: the hard case, in both domains |
| `build_jpeg_arms.py` | CLI | The JPEG arms: the gap round 2 named and could not fill |
| `build_core_tier.py` | CLI | Build every arm of a tier, with bounded parallelism and a resumable plan |
| `payloadsweep.py` | CLI | Detection against payload rate, at a fixed image size |
| `sizesweep.py` | CLI | Does a detector's 448x448 input resize destroy the signal it looks for? |

### Controls, which exist to fail

A control that cannot fail is worse than no control, because it converts an
unmonitored risk into a monitored one nobody re-examines. Each of these is here
because a measurement was once wrong in exactly the way it detects.

| Module | | What it does |
|---|---|---|
| `structural.py` | CLI | Data appended after the PNG end-of-image marker: the floor every tool should catch |
| `null_control.py` | CLI | Run the classifier against a problem with no signal in it, and check it fails |
| `recompression_control.py` | CLI | Separate "the detector saw the payload" from "the detector saw the encoder" |
| `container_probe.py` | CLI | Where can you hide data in a file without the pixels changing at all? |

### Repair, for arms already built

| Module | | What it does |
|---|---|---|
| `repair_writer_matched_pairs.py` | CLI | Give an already-built arm the clean half it should have had |
| `repair_jpeg_pair_passes.py` | CLI | Give the clean JPEG half the second writer pass its stego twin already had |
| `rebuild_replaced_covers.py` | CLI | Invalidate the arm files derived from covers that were replaced |
| `stamp_source_digests.py` | CLI | Write the cover digest onto every arm row, once the rebuild has landed |

### Scoring

| Module | | What it does |
|---|---|---|
| `score_arms.py` | CLI | Score a built corpus with the detector under test and a reference detector |
| `panel_scores.py` | CLI | Score a corpus with the established detector panel, and report per arm |
| `aletheia_scores.py` | CLI | Run inside the Aletheia container and emit the numbers its CLI throws away |
| `fld_ensemble.py` | lib | The FLD ensemble classifier, which turns rich-model features into a detector |
| `rich_model_baseline.py` | CLI | Train a rich-model detector on one arm, so "nobody can see this" is measured |
| `emit_results.py` | CLI | Turn a scored corpus into `result-v1` documents |

### Release

| Module | | What it does |
|---|---|---|
| `tiers.py` | lib | Select covers in tier order, so a smaller tier is a prefix of a larger one |
| `pack_tier.py` | CLI | Pack a tier into sharded archives, so the corpus is downloadable |
| `pack_arms.py` | CLI | Pack the stego arms into shards, so the other half of the corpus ships too |
| `publish_tier.py` | CLI | Turn a packed tier into a release, from one source of truth |
| `release_metadata.py` | CLI | Write the files a published dataset needs and this one was missing |
| `validate_croissant.py` | CLI | Check a Croissant record before it is published |
| `verify_release.py` | CLI | Every invariant this corpus must satisfy before it is published, in one run |
| `make_fixtures.py` | CLI | Build the self-test fixtures: one image that is clean, one that is not |

### The way in

| Module | | What it does |
|---|---|---|
| `cli.py` | CLI | `pentimento`, one entry point to all of these, found by import not by a list |

## Two pieces of the corpus pipeline that are deliberately NOT here

They look like strays. They are not, and consolidating them would break the
release.

**`../tools/release/upload_tier.py`** is the last step of the chain and lives
outside the package on purpose. `upload-in-container.sh` bind-mounts **that one
file** into a container that holds write tokens for four public archives and
runs unattended for six hours with `--cap-drop=ALL` and a read-only root:

    -v "$REPO/tools/release/upload_tier.py:/upload_tier.py:ro"

So it has to be a standalone module with no package imports, because nothing
else is mounted for it to import. Moving it into `generators/` would break the
upload outright, and mounting the whole package instead would widen what a
credential-holding container can read, which is the one thing that container
exists to keep small. It is separate because the blast radius is small, and
that is worth more than tidiness.

**`../tools/ci/tier_smoke.py`** runs on three operating systems in CI, because
the corpus is built on Linux and the paper says anyone can rebuild it. Docker
cannot check that: a Linux host cannot run Windows or macOS containers, so the
only honest test is a runner of each kind. It belongs with CI, not with the
builders.

Both are reachable from the release chain and neither is orphaned. If a future
tidying pass wants to fold them in, read this section first and then read
`upload-in-container.sh`.

## The order things run in, and why it is not the order you would guess

    fetch_commons ─▶ dedup ─▶ cover_quality ─▶ manifest_repair
                                                     │
                            audit_pd_licences ◀──────┤
                                    │                │
                            select_unpublishable     │
                                    │                │
                            backfill_covers ─────────┘
                                    │
                    ┌───────────────┴───────────────┐
                    ▼                               ▼
            build_jpeg_arms                 build_adaptive_arms
        (also WRITES the JPEG                (READS that pool,
         cover pool the other                 by POSITION)
         one reads)
                    └───────────────┬───────────────┘
                                    ▼
                          stamp_source_digests
                     (arms cannot prove where they
                      came from until this has run,
                      and verify_release refuses a
                      corpus whose arms cannot)
                                    │
                                    ▼
                     pack_tier ─▶ pack_arms ─▶ publish_tier
                                    │
                            release_metadata ─▶ validate_croissant
                                    │
                             verify_release

**The arrow that bites.** `build_adaptive_arms.py` indexes the JPEG cover pool
positionally: index *i* is the *i*-th name in sorted order, which equals cover
*i* only while the pool is dense. Build the adaptive arms before the JPEG cover
pool is complete and every cover after the first gap is paired with coefficients
it never came from, while every digest matches and every count is right. Both
tools now refuse a pool that is not dense, but the ordering is still the thing
to get right rather than the check.

## Reproducibility

Every generator is seeded and writes `manifest.jsonl` beside its output, one
record per pair, carrying the source filename, the parameters, the number of
samples actually changed, and a sha256 of both files.

Three things had to be fixed before "re-running with the same seed reproduces
the corpus" was true rather than merely claimed:

- **Payloads** came from one shared generator consumed in a loop that skips on
  resume and on every cover a tool refuses, and outguess refuses 1,884 of
  10,000. They are now derived per image by hash, in `payloads.py`.
- **Container base images** were floating tags rather than digests. All nine are
  now pinned, and `../tools/IMAGES.md` records the exact images that built v1.
- **The Python environment was never written down.** `requirements.txt` pinned
  seven packages; the build machine had fifty-one. `conseal` compiles its cost
  maps with **numba**, so the function deciding which pixels carry the payload
  is JIT-compiled machine code and a different numba or llvmlite can move a cost
  by an ulp and change which pixel is chosen. Neither was named anywhere. See
  `../requirements.lock`.

## Tests

    python3 -m unittest discover -s generators -p 'test_*.py'

Fifteen test modules, 361 tests. They run without containers, without network
and without the corpus: anything needing a real embedder skips with a reason
rather than passing vacuously.

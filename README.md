# stegobench

A reproducible benchmark for image steganalysis: build a labelled corpus, run
detectors over identical bytes, and report numbers somebody else can check.

Most published steganalysis results cannot be reproduced by the people reading
them, for two reasons that have nothing to do with the science. The corpora are
not redistributable, and the pairing discipline is described rather than
enforced. This is an attempt at both halves.

## What is here

| Piece | What it does |
|---|---|
| `generators/fetch_commons.py` | Builds a cover corpus from Wikimedia Commons under permissive licences, with per-file provenance |
| `generators/dedup.py` | Perceptual-hash deduplication, with a corroboration rule for low-texture images |
| `generators/build_adaptive_arms.py` | Stego arms for HUGO, WOW, S-UNIWARD, HILL, MiPOD, J-UNIWARD and UERD |
| `generators/build_jpeg_arms.py` | Arms for steghide and outguess, plus an appended-data control |
| `generators/score_arms.py` | Scores a corpus and reports AUC, detection at a fixed false-alarm rate, and verdict rate |
| `generators/panel_scores.py` | The same, for a panel of reference detectors |
| `generators/fld_ensemble.py` | A rich-model classifier, for measuring what the state of the art reaches |
| `generators/pack_tier.py` | Packs a tier into WebDataset shards, reproducibly |
| `generators/publish_tier.py` | Derives every hosting platform's metadata from the per-file manifest |

## The two ideas it is built around

**A pair differs only in the payload.** If the clean image and the stego image
differ in any other way, a resave, a quantisation table, a stripped timestamp,
then a detector learns the artefact instead of the hiding. That is not a
hypothetical: it voided a whole round of measurements here, because outguess
re-encodes at quality 75 whatever it was given and the clean half had been
written at 95. Both halves now come off the same writer, and the arms that did
not are kept as the demonstration.

**A verdict cannot produce an ROC curve.** Several detectors, including some
reference implementations, compare their own estimate to a threshold and print a
sentence. The estimate is what a comparison needs, so where a tool computes one
and discards it, the harness calls the same function and keeps the number.

## Installing

Python 3.14 and, for the Rust crates, the toolchain pinned in
`rust-toolchain.toml`. Everything below runs from a clean checkout.

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

That is enough to build a corpus, embed every arm and score it.
`requirements-optional.txt` holds four things that are each needed for exactly
one purpose and are each large or awkward to install: Aletheia for the
reference detector and the SRM features, matplotlib for charts, `lir` for the
cross check against an independent likelihood ratio implementation, and the
Hansken SDK for the extraction plugin. Nothing in the core needs them, and the
tests that do will skip rather than fail when they are absent.

Versions are pinned to the ones that produced the published numbers. A
benchmark whose dependency set floats is a benchmark whose numbers move without
anybody touching it.

```sh
.venv/bin/python -m pytest                 # python
cargo test --workspace                     # rust
```

## Reproducing a result

```sh
# 1. covers, with provenance and licence per file
python3 generators/fetch_commons.py --out covers/ --count 1000 \
    --dedup-db dedup.sqlite3 --licences permissive

# 2. arms
python3 generators/build_adaptive_arms.py --covers covers/ --out arms/ \
    --count 1000 --schemes suniward --rates 0.4

# 3. score, against whichever detector you are testing
python3 generators/score_arms.py --corpus arms/ --endpoint http://HOST:PORT/your-detector-api
```

There is deliberately no default endpoint. A benchmark that ships one address as
a default scores against whatever answers on it.

## Pentimento

The cover corpus this was built to produce. See `docs/pentimento.md` for what it
is and `docs/distribution.md` for how it is packaged and published, including
the thing it is important to say first: **it is a JPEG-decompressed spatial
corpus and is not comparable to BOSSbase.**

`docs/cover-source-licensing.md` records why almost every existing steganalysis
corpus cannot be redistributed, which is the reason this one exists.

## Status

The harness is working and in use. The first corpus tier is complete: 29 arms,
344,348 pairs, built in a single 22 hour run with every arm resumable and every
file checksummed.

What that does and does not mean is worth being exact about. The generators,
the pairing discipline and the scoring are exercised on real corpora and the
numbers in the results directory came out of them. The packaging and
publication path for the corpus itself is built but has not yet carried a
public release, so treat `pack_tier.py` and `publish_tier.py` as the least
travelled code here.

## Licence

AGPL-3.0-or-later. See `LICENSE`.

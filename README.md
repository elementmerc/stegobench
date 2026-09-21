# Stegobench

A reproducible benchmark for image steganalysis: build a labelled corpus, run
detectors over identical bytes, and report numbers somebody else can check.

Most published steganalysis results cannot be reproduced by the people reading
them, for two reasons that have nothing to do with the science. The corpora are
not redistributable, and the pairing discipline is described rather than
enforced. This is an attempt at both halves.

**What you need:** Python 3.14. The Rust crates want the toolchain pinned in
`rust-toolchain.toml`, and you do not need them to build or score a corpus.

```sh
git clone https://github.com/elementmerc/stegobench
cd stegobench
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

Full documentation, including a quickstart and the limitations worth reading
before you quote a number: **[docs/](docs/index.md)**, or the built site once
it is published.

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

The clone and the two commands at the top of this file are the whole install.
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
.venv/bin/python -m unittest discover -s generators -p "test_*.py"
.venv/bin/python -m unittest discover -s tools/release -p "test_*.py"
cargo test --workspace   # only if you want the Rust side
```

Those are the two Python suites CI runs, so a green pair here is a green pair
there. Tests needing something from `requirements-optional.txt` skip rather
than fail.

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

The corpus this was built to produce. It has
[its own repository](https://github.com/elementmerc/pentimento), because a
citation should point at the dataset rather than at the tool that made it.

`docs/design/pentimento.md` covers what it is and `docs/design/distribution.md`
how it is packaged and published, including the thing to say first: **it is a
JPEG-decompressed spatial corpus and is not comparable to BOSSbase.**

`docs/design/cover-source-licensing.md` records why almost every existing
steganalysis corpus cannot be redistributed, which is the reason this one
exists, and `docs/design/matched-pairs.md` records the defect class that broke
two arms before a structural check caught it.

## Status

The harness is working and in use. The first corpus tier is complete: 35 stego
arms and 4 clean ones, 344,348 pairs, built in a single 22 hour run with every
arm resumable and every file checksummed.

What that does and does not mean is worth being exact about. The generators,
the pairing discipline and the scoring are exercised on real corpora and the
numbers in the results directory came out of them. The packaging and
publication path for the corpus itself is built but has not yet carried a
public release, so treat `pack_tier.py` and `publish_tier.py` as the least
travelled code here.

## Licence

AGPL-3.0-or-later. See `LICENSE`.

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

Working, and not yet released. The corpus and the harness are built and
measured; the first public release is pending.

## Licence

AGPL-3.0-or-later. See `LICENSE`.

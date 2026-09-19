# Quickstart

Ten minutes, from a clean checkout to a scored arm. Everything here runs on
Linux, macOS and Windows; CI builds a small tier on all three on every push.

## What you need

Python 3.14. For the Rust crates, the toolchain pinned in
`rust-toolchain.toml`; you do not need it to build or score a corpus.

```sh
git clone https://github.com/elementmerc/stegobench
cd stegobench
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

## Check it works

```sh
.venv/bin/python -m unittest discover -s generators -p "test_*.py"
cargo test --workspace                     # only if you want the Rust side
```

## Build something small and score it

```sh
# 1. covers, with provenance and a licence per file
python3 generators/fetch_commons.py --out covers/ --count 1000 \
    --dedup-db dedup.sqlite3 --licences permissive

# 2. one arm
python3 generators/build_adaptive_arms.py --covers covers/ --out arms/ \
    --count 1000 --schemes suniward --rates 0.4

# 3. score, against whichever detector you are testing
python3 generators/score_arms.py --corpus arms/ \
    --endpoint http://HOST:PORT/your-detector-api
```

There is deliberately no default endpoint. A benchmark that ships one address
as a default scores against whatever answers on it.

## Or skip the building

If you want the corpus rather than the machinery, the Core tier is already
built and published: 10,000 covers and 344,348 matched pairs, every image with
its licence attached. See [Pentimento](/pentimento).

## The one thing to read before you quote a number

[Read this before quoting a number](/guide/limits). The short version: the
corpus is JPEG-decompressed, so its numbers do not belong in the same table as
BOSSbase numbers, and a random train/test split over it will flatter your
classifier by putting a cover in training and its own stego copy in test.

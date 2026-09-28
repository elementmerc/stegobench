# Quickstart

## Install

Python 3.12 or newer, which is what `pyproject.toml` requires; CI runs 3.14.
The Rust crates need the toolchain pinned in
`rust-toolchain.toml`; you do not need it to build or score a corpus.

```sh
git clone https://github.com/elementmerc/stegobench
cd stegobench
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

That is enough to build a corpus, embed every arm and score it.

| | |
|---|---|
| `requirements.txt` | Everything the core needs, pinned to the versions that produced the published numbers |
| `requirements-optional.txt` | matplotlib for charts, `lir` for the likelihood-ratio cross check, `mlcroissant` for validating the dataset record, `webdataset` for reading a packed shard, the Hansken SDK for the extraction plugin, and a note on where to get Aletheia, the reference detector, which isn't on PyPI under that name. Tests that need any of them skip rather than fail |

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
scores against whatever answers on it.

## Or skip the building

A corpus ships in the box, a smaller one than you can quote a number from, and
a bigger one built with this harness is already published with its own
documentation: see [Getting a corpus](/guide/getting-a-corpus) and
[Pentimento](https://github.com/elementmerc/pentimento).

## Before you quote a number

[Limitations](/guide/limits).

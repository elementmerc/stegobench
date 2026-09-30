# Quickstart

Two halves live in this repository and they're installed separately. The
harness, `stegobench`, is the Rust tool that runs a detector over labelled
images and reports how often it was right. The generators, `pentimento`, are
the Python programs that build a labelled corpus in the first place.

Start with the harness. You only need the second half if you want to build your
own corpus.

## Part one: the harness

### Install it

You need the Rust toolchain pinned in `rust-toolchain.toml`.

```sh
git clone https://github.com/elementmerc/stegobench
cd stegobench
cargo install --path crates/stegobench-cli
```

There's nothing to configure. The registry of detectors, embedders and corpora
is compiled into the binary as a fallback, so `stegobench` works from any
directory on a machine with no checkout on it.

### See what it is

Type the bare name and it tells you in three lines, rather than printing three
screens of help at somebody who's still deciding whether this is the tool they
want.

```sh
stegobench
```

The important thing it says: Stegobench measures **detectors**. It doesn't
examine your own images. Those are opposite directions, and
`stegobench help scope` spells out the difference and where to go for the other
one.

### See what this installation can run

```sh
stegobench list detectors
stegobench list corpora
```

Both lists are read from the registry rather than written by hand, so they
can't drift away from what the tool will actually do. Each detector is either a
container pinned by image digest (sandboxed, no network) or a program you
installed yourself.

### Check the machine can do it

```sh
stegobench doctor
```

`doctor` looks for every registered tool, prints the exact line to type for
each one that's missing, and asks each installed one to flag a known planted
signal *and* clear a known clean image. Both directions, because a tool that
answers "stego" to everything passes a one-sided check. Add `--no-selftest` for
the fast version, which can tell you what's present and can't tell you what
works.

It exits 8 when the machine can measure nothing at all.

### Get something to score

A starter corpus travels inside the binary, so a fresh install has something to
point at before it has downloaded anything:

```sh
stegobench fetch stegobench-starter --tier nano
```

That writes the corpus to `./stegobench-starter`; `--dest` puts it somewhere
else. It's six synthetic greyscale covers and the twelve LSB stego images made
from them, eighteen images in total, under CC0. Each image has a JSON record
beside it, and there's a manifest, a README and the script that built them, so
`fetch` reports thirty nine files written. You don't need a clone of this
repository for any of it; the bytes are inside the binary you installed. (If
you happen to have a clone, they're also at `corpora/starter`, and they're the
same bytes.)

**It's a demonstration, not a measurement.** Six covers is few enough that one
image moves the AUC (area under the ROC curve) by a tenth, and the covers are
generated rather than photographed, so nothing calibrated on it transfers to
real pictures. It's there so you can watch the machinery work. For a corpus you
can quote a number from, see [Getting a corpus](/guide/getting-a-corpus).

### Find out what a run would cost

```sh
stegobench plan score --corpus stegobench-starter --detector all
```

`plan` takes the command you'd actually type, flags and all, so it can't
describe a different run from the one that would happen. It counts the corpus
rather than guessing from its size, gives a per detector estimate and a worst
case with every image hitting the timeout, and says whether the run would come
out `named` or `custom`.

### Score it

```sh
stegobench score --corpus stegobench-starter --detector zsteg --out result.json
```

It asks the detector about every image, writes each answer to a records file as
it goes, and emits a validated `result-v1` document naming the exact bytes it
measured. Interrupt it and run the same command again and it picks up where it
stopped.

Useful flags:

| Flag | What it does |
|---|---|
| `--detector all` | every registered detector over the same bytes in one pass |
| `--split test` | score only the held-out half, which is where a trained detector's number has to come from |
| `--limit N` | stop after N items, for a smoke test. Marks the result `custom` |
| `--timeout SECONDS` | how long one image gets before the detector is killed. Default 60 |
| `--trained-on ID` | record which corpus this detector was trained on, because a detector scored on what it trained on isn't being measured |

Exit code 0 means every detector asked for produced a result. 3 means at least
one was skipped as unavailable and 4 means at least one failed while running,
and you get those even when the others succeeded, so 0 never quietly means
"some of them".

### Read the result

```sh
stegobench report result.json --format markdown
stegobench validate result.json
stegobench verify result.json --corpus stegobench-starter
```

`report` renders the conditions into every row, so a figure can't be lifted out
without them, and orders rows by arm then detector rather than by score,
because it isn't a ranking. `validate` checks a document against its schema and
exits 6 with every problem named, not just the first. `verify` recomputes the
corpus digest from the images on disk and exits 5 when the document and the
corpus turn out not to be about each other.

[Reading a result](/guide/reading-a-result) goes through the fields.

### Where the reasoning lives

Anything too long for a `--help` line is a help topic:

```sh
stegobench help            # lists them
stegobench help pairing
```

The topics are `scope`, `pairing`, `splits`, `licences`, `plugins`, `results`
and `reports`.

## Part two: building your own corpus

Skip this unless you want to build a labelled corpus rather than score one.
This is the Python half, it installs separately, and it's the older and more
exercised of the two: it's what produced the published Pentimento corpus.

### Install it

Python 3.12 or newer, which is what `pyproject.toml` requires; CI runs 3.14.

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

| | |
|---|---|
| `requirements.txt` | Everything the core needs, pinned to the versions that produced the published numbers |
| `requirements-optional.txt` | matplotlib for charts, `lir` for the likelihood-ratio cross check, `mlcroissant` for validating the dataset record, `webdataset` for reading a packed shard, the Hansken SDK for the extraction plugin, and a note on where to get Aletheia, the reference detector, which isn't on PyPI under that name. Tests that need any of them skip rather than fail |

### Check it works

```sh
.venv/bin/python -m unittest discover -s generators -p "test_*.py"
cargo test --workspace                     # the Rust side, if you want it too
```

### Build something small and score it

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

There's deliberately no default endpoint. A benchmark that ships one address
scores against whatever answers on it.

[Build a corpus](/guide/build-a-corpus) is the long version.

## Before you quote a number

[Limitations](/guide/limits).

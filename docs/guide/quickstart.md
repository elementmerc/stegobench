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
stegobench score --corpus stegobench-starter --detector aletheia-rs --out result.json
```

It asks the detector about every image, writes each answer to a records file as
it goes, and emits a validated `result-v1` document naming the exact bytes it
measured. Interrupt it and run the same command again and it picks up where it
stopped.

The headline line of that run:

```
aletheia-rs      AUC 1.0000 [1.0000, 1.0000]  6 clean / 12 stego / 0 unanswered  result.json
```

A perfect score, because plain LSB in a synthetic greyscale cover is the
easiest thing in this field to spot. Six clean images is also too few to be
wrong on, which is why the run says so itself. `aletheia-rs` is the detector
`doctor` is most likely to have found on a fresh machine; if it didn't,
`doctor` printed the line to type, and `--detector all` scores with whatever
this installation can actually run.

Not every detector suits every corpus. `zsteg` reads the structural tricks
that hide data in PNG channel and bit-plane orderings, so on these images it
answers the same thing eighteen times and lands on an AUC of exactly 0.5. That
figure is 0.5 by construction rather than by measurement, and the run says so
rather than leaving it to be read as chance.

Useful flags:

| Flag | What it does |
|---|---|
| `--detector all` | every registered detector over the same bytes in one pass |
| `--split test` | score only the held-out half, which is where a trained detector's number has to come from |
| `--limit N` | stop after N items, for a smoke test. Marks the result `custom` |
| `--timeout SECONDS` | how long one image gets before the detector is killed. Default 60 |
| `--jobs N` | how many images to score at once. Default 1, one at a time, because several workers competing for one machine can make a tool fail in ways that look like a result. Raise it slowly |
| `--keep-raw` | keep what the detector printed for every image, in `<records>.raw.jsonl`. Several times the size of the records file, and what you want while writing an adapter |
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
.venv/bin/pip install -r requirements.lock
.venv/bin/pip install -e . --no-deps
```

That's the one recipe. `requirements.lock` is every package that was installed
on the machine that built the published corpus, `numba` and `llvmlite`
included. Those two compile the code that decides which pixels carry the
payload, so a different version of either can change the bytes a build
produces. `--no-deps` stops pip resolving round the lock it was just given.

`pip install -e .` on its own also works and gives you the same command, but it
floats that compiler stack. A build run outside the locked set warns on stderr
before it starts.

The lock records one machine, Python 3.14 on Linux x86_64. On macOS or Windows,
install `requirements.txt` instead: your build will be internally consistent
and will not be byte-identical to the published corpus.

| | |
|---|---|
| `requirements.lock` | Every package the published build actually had. Install this one |
| `requirements.txt` | The seven packages this project chose, without the compiler stack underneath them |
| `requirements-optional.txt` | matplotlib for charts, `lir` for the likelihood-ratio cross check, `mlcroissant` for validating the dataset record, `webdataset` for reading a packed shard, the Hansken SDK for the extraction plugin, and a note on where to get Aletheia, the reference detector, which isn't on PyPI under that name. Tests that need any of them skip rather than fail |

### Check it works

```sh
.venv/bin/pentimento --version             # record this beside any corpus you build
.venv/bin/python -m unittest discover -s generators -p "test_*.py"
cargo test --workspace                     # the Rust side, if you want it too
```

### Build something small and score it

```sh
# 1. covers, with provenance and a licence per file
.venv/bin/pentimento fetch-commons --out covers/ --count 1000 \
    --dedup-db dedup.sqlite3 --licences permissive

# 2. assign the tier order over the finished cover set. Nothing below
#    this line runs without it
.venv/bin/pentimento manifest-repair covers/manifest.jsonl

# 3. one arm
.venv/bin/pentimento build-adaptive-arms --covers covers/ --out arms/adaptive \
    --count 1000 --schemes suniward --rates 0.4

# 4. score, against whichever detector you are testing
.venv/bin/pentimento score-arms --corpus arms/adaptive \
    --endpoint http://HOST:PORT/your-detector-api
```

There's deliberately no default endpoint. A benchmark that ships one address
scores against whatever answers on it.

[Build a corpus](/guide/build-a-corpus) is the long version.

## Before you quote a number

[Limitations](/guide/limits).

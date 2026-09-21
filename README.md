# stegobench

A reproducible benchmark for steganalysis: run detectors and embedders as
sandboxed plugins, score them against a labelled corpus, and get back a
versioned JSON document that names the exact bytes it was measured on.

Most published steganalysis results can't be checked by the people reading
them. The corpora usually can't be redistributed, and the discipline that
keeps a measurement honest (a clean image and its stego twin must differ in
nothing but the payload, a cover and its stego twin must land on the same
side of a train/test split) is described in a paper rather than enforced by
the tool that produced the number. stegobench is an attempt at fixing both:
[Pentimento](https://github.com/elementmerc/pentimento) is the redistributable
corpus, and this repository is the harness.

## What's here today, honestly

Two halves, at different stages:

- **A Rust command-line tool** (`stegobench`) that reads a registry of
  detectors, embedders and corpora, checks whether this machine can run them, and
  will score a corpus against them. `schema`, `validate`, `list`, `describe`,
  `doctor`, `completions` and `help` are built and tested. `plan` and `score`
  are in the command tree and documented, but not built yet: running either
  today exits with a message saying exactly that, rather than pretending to
  work.
- **Python generators** (`generators/`) that build a labelled corpus:
  fetching covers with provenance, embedding stego arms, packing shards, and
  scoring against a detector's HTTP endpoint. This half is older, working,
  and what built the corpus behind the numbers in `results/`.

The two halves aren't merged yet. The registry (`plugins/registry/`) is real
and the Rust `list`/`describe`/`doctor` commands read it live; the actual
running of a detector against a corpus, end to end, from the Rust binary, is
the next piece of work, not a finished feature.

## Three commands to try

```sh
# Build the tool
cargo build --release -p stegobench-cli

# What can this installation run, and where did that answer come from?
# (Read live from plugins/registry/, so it can't go stale like a README can.)
./target/release/stegobench list detectors --json

# Is this machine actually able to run what the registry claims?
# (Checks for each tool's container or binary, and where present, asks it
# to score a known planted signal and a known clean fixture.)
./target/release/stegobench doctor

# What does a measurement look like, structurally?
./target/release/stegobench schema result-v1
```

Every subcommand accepts `--json`, which puts machine-readable output on
stdout and leaves progress and human text on stderr, so
`stegobench doctor --json | jq` works while you can still watch it run.

Conceptual reasoning that doesn't fit on a `--help` line lives behind
`stegobench help <topic>`: try `stegobench help pairing` or
`stegobench help plugins`.

## Adding your own detector

A detector or embedder is a TOML file under `plugins/registry/`, not code
wired into the harness. The shortest version, for a tool that's a container
pinned by digest:

```toml
name = "my-detector"
kind = "detector"
licence = "MIT"

[image]
reference = "ghcr.io/you/my-detector@sha256:..."   # a tag is refused
size_mb = 200
bundled = false

[emits]
output = "score"          # or "verdict" if it only says yes or no

[accepts]
formats = ["png"]

[selftest]
must_detect = "fixtures/lsb-0.4bpp.png"   # it must flag this
must_clear = "fixtures/clean.png"          # and clear this
```

Both fixtures under `[selftest]` are required. A tool that answers "stego" to
everything, or "clean" to everything, would otherwise pass a one-sided check;
`stegobench doctor` runs both directions before believing a tool works. See
`stegobench help plugins` for the full reasoning, and an existing entry under
`plugins/registry/` (`steghide.toml` is a short one) for a worked example
with a container that needs an embed/extract round trip proven too.

## Registering a corpus

A corpus is a TOML file under `plugins/registry/corpora/`, and it is a
separate shape from a detector's for a plain reason: a corpus is data you
point at, a plugin is code you run. It has a licence, a download route and a
cover count; it has no container image, no argv and no self-test.

`stegobench list corpora` and `stegobench describe <id>` read them the same
way they read a tool, so a user sees one registry.

```toml
id = "example"
name = "Example Corpus"
description = "What it is, for somebody who has never heard of it."

[licence]
status = "verified"        # or "unverified", or "none-granted"
spdx = "CC-BY-SA-4.0"
url = "https://creativecommons.org/licenses/by-sa/4.0/legalcode"
verified_on = "2026-09-21"
source = "the deposit's own terms page, read on that date"
redistribution = "permitted"    # or "forbidden", or "unknown"
redistribution_reason = "Why, in one sentence. Required in every case."
attribution_required = true
share_alike = true

[obtain]
doi = "10.0000/example"

[properties]
base_images = 100
formats = ["png"]
paired = true
```

Two rules the file is held to, and both come from real damage. A licence is
`verified` only with an identifier, a link to the licence text, a date and a
note saying what was read; anything else is `unverified` or `none-granted`,
and carrying a licence name beside either of those is refused. Mirrors of
well-known corpora are labelled MIT, Apache 2.0 and CC0 where the original
granted none of them, and a guess written into a metadata field is how that
starts.

**Redistribution is a separate field from the licence**, because it is a
separate question. A corpus you may use is not always one you may publish,
and republication is refused outright on terms nobody has read.

## Building a corpus (the Python half)

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt

# 1. covers, with provenance and licence recorded per file
python3 generators/fetch_commons.py --out covers/ --count 1000 \
    --dedup-db dedup.sqlite3 --licences permissive

# 2. an embedding arm
python3 generators/build_adaptive_arms.py --covers covers/ --out arms/ \
    --count 1000 --schemes suniward --rates 0.4

# 3. score it against a detector you're testing
python3 generators/score_arms.py --corpus arms/ \
    --endpoint http://HOST:PORT/your-detector-api
```

There's deliberately no default `--endpoint`. A benchmark that ships one
address as a default scores against whatever happens to answer on it.

`requirements-optional.txt` holds four things each needed for exactly one
purpose: Aletheia for the reference detector and its rich-model features,
matplotlib for charts, `lir` for a cross-check against an independent
likelihood-ratio implementation, and the Hansken SDK for the extraction
plugin. Nothing in the core needs them, and tests that do skip rather than
fail when they're absent.

## Status: what's measured, what isn't

**Measured and working:** the corpus generators (deduplication, licence
tracking, embedding, packing), and the scoring loop against an HTTP endpoint.
The first corpus tier is complete: 35 stego arms and 4 clean ones, 344,348
pairs, built in a single 22 hour run with every arm resumable and every file
checksummed.

**Built and tested, Rust side:** the tool registry and its validation
(a mutable image tag is refused, not merely discouraged), the three
published schemas (`result-v1`, `run-v1`, `manifest-v1`, all generated from
the Rust types that write them, never hand maintained), and `doctor`'s
two-sided self-test.

**Not built yet:** `stegobench plan` (a cost estimate before you commit to a
long run) and `stegobench score` (actually running a detector against a
corpus from the Rust binary). Both are in the command tree today so the
vocabulary is fixed before anything depends on it, and both refuse clearly
with exit code 8 rather than doing something partial.

**Twelve tools are registered** under `plugins/registry/` today: five
embedders (steghide, outguess, openstego, stegosuite, hstego) and seven
detectors (Aletheia's SPA, RS and rich-model estimators, StegExpose, zsteg,
plus Stegcore and StegaShield as subjects rather than references).
`stegoveritas` has never built against current dependencies and is
deliberately not registered or listed anywhere as if it worked; see
`DEFERRED.md` for the detail rather than a dead reference here.

**stegoveritas, F5, jsteg and jphide are not present.** They're candidates
for later, not silently dropped: naming them here rather than letting a
reader discover the gap is the point of this section existing at all.

## Pentimento

The corpus this harness was built to score. It has
[its own repository](https://github.com/elementmerc/pentimento), because a
citation should point at the dataset rather than at the tool that made it.
Read `docs/design/pentimento.md` for what it is; the sentence to say first
about it is that it's a JPEG-decompressed spatial corpus and is not
comparable to BOSSbase.

## Running the test suites

```sh
cargo test --workspace
.venv/bin/python -m unittest discover -s generators -p "test_*.py"
.venv/bin/python -m unittest discover -s tools/release -p "test_*.py"
```

These are the suites CI runs, so a green result here is a green result
there. Python tests needing something from `requirements-optional.txt` skip
rather than fail.

## Licence

AGPL-3.0-or-later. See `LICENSE`.

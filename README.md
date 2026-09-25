# Stegobench

A reproducible benchmark for steganalysis: run detectors and embedders as
sandboxed plugins, score them against a labelled corpus, and get back a
versioned JSON document that names the exact bytes it was measured on.

Most published steganalysis results can't be checked by the people reading
them. The corpora usually can't be redistributed, and the discipline that
keeps a measurement honest (a clean image and its stego twin must differ in
nothing but the payload, a cover and its stego twin must land on the same
side of a train/test split) is described in a paper rather than enforced by
the tool that produced the number. Stegobench is an attempt at fixing both:
[Pentimento](https://github.com/elementmerc/pentimento) is the redistributable
corpus, and this repository is the harness.

## What's here today, honestly

Two halves, at different stages:

- **A Rust command-line tool** (`stegobench`) that reads a registry of
  detectors, embedders and corpora, checks whether this machine can run them,
  and scores a corpus against them. Every command in the tree is built and
  tested: `schema`, `validate`, `verify`, `list`, `describe`, `doctor`,
  `plan`, `score`, `completions` and `help`.
- **Python generators** (`generators/`) that build a labelled corpus:
  fetching covers with provenance, embedding stego arms, packing shards, and
  scoring against a detector's HTTP endpoint. This half is older, working,
  and what built the corpus behind the numbers in `results/`.

The two halves aren't merged yet. The registry (`plugins/registry/`) is real
and the Rust `list`/`describe`/`doctor` commands read it live, and `score`
runs a registered detector over a corpus directory end to end. What it can't
do yet is score a registered tier: it reads an unpacked directory, so a packed
tier has to be extracted first and the run is marked `custom`. That, and
building a corpus from the Rust side, is what the Python half still owns.

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
`stegobench help results`, which is how to judge a number somebody else
produced.

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
bundled = true            # derived from the size, not chosen: true at or
                          # below 750 MB, and the registry refuses a file
                          # whose flag disagrees with its own size

[emits]
output = "score"          # or "verdict" if it only says yes or no

[accepts]
formats = ["png"]

[selftest]
must_detect = "fixtures/lsb-0.4bpp.png"   # it must flag this
must_clear = "fixtures/clean.png"          # and clear this
```

A tool is registered one of two ways, never both, and `stegobench list` names
which one each tool uses:

| Route | What it means for you |
|---|---|
| `[image]` | A container, pinned by digest. It runs with no network, and two machines run identical bytes. You need a container runtime |
| `[binary]` | A program you installed. No sandbox, and the hash pins the file on your machine rather than bytes anybody can pull |

A `[binary]` entry can add `platforms = ["windows"]` where the tool only exists
on some systems. `stegobench doctor` then says it cannot run here, instead of
reporting it as missing and sending you to look for a package that does not
exist for you. Leaving it out means nobody has said, which is not the same as
saying it runs everywhere.

Both fixtures under `[selftest]` are required. A tool that answers "stego" to
everything, or "clean" to everything, would otherwise pass a one-sided check;
`stegobench doctor` runs both directions before believing a tool works. See
`stegobench help plugins` for the full reasoning, and an existing entry
(`plugins/registry/embedders/steghide.toml` is a short one) for a worked
example with a container that needs an embed/extract round trip proven too.

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

`requirements-optional.txt` holds one thing per purpose: matplotlib for
charts, `lir` for a cross-check against an independent likelihood-ratio
implementation, `mlcroissant` for validating the dataset record, `webdataset`
for reading a packed shard, and the Hansken SDK for the extraction plugin. It
also tells you where to get Aletheia, the reference detector, which isn't on
PyPI under that name and has to be installed from source. Nothing in the core
needs any of them, and tests that do skip rather than fail when they're
absent.

## Status: what's measured, what isn't

**Measured and working:** the corpus generators (deduplication, licence
tracking, embedding, packing), and the scoring loop against an HTTP endpoint.
The first corpus tier is complete: 35 stego arms and 4 clean ones, 344,357
pairs, built in a single 22 hour run with every arm resumable and every file
checksummed.

**Built and tested, Rust side:** the tool registry and its validation
(a mutable image tag is refused, not merely discouraged), the three
published schemas (`result-v1`, `run-v1`, `manifest-v1`, all generated from
the Rust types that write them, never hand maintained), and `doctor`'s
two-sided self-test.

**`stegobench score` runs.** Point it at a directory of samples and a
registered detector and it asks about every one, writing each answer as it
goes, then emits a validated `result-v1` document. The run resumes: if it is
interrupted, running the same command again picks up where it stopped rather
than starting over, and it refuses to continue if the corpus changed under the
records rather than filing answers against the wrong images.

It also checks the two rules rather than declaring them. A corpus that puts a
cover and its stego twin on opposite sides of a train and test split stops the
run instead of producing an inflated number nobody could spot afterwards. And
for every stego image that names the cover it came from, the headers of both
are compared: a difference in format, size, bit depth or channel count means
something other than the payload changed, and the result says so and names the
images. Matching headers prove nothing on their own, so the result distinguishes
"looked and found nothing" from "could not look".

Point it at a registered corpus and it checks that claim rather than taking
it:

```sh
stegobench score --corpus ./pentimento-nano --corpus-id pentimento-core \
    --detector zsteg
```

A run is marked `named` only when the corpus entry declares the digest of its
records and the directory in front of the tool matches it. A directory that
doesn't match is refused before anything is scored, because you asserted
something about those bytes that isn't true of them. Most corpora have no such
digest yet, and those runs are `custom`, carry the registered name and tier,
and say in one line why. `custom` is a perfectly good run: it's comparable with
itself rather than with somebody else's.

One honest limit today: it reads an UNPACKED corpus directory, so a packed tier
has to be extracted first.

**`stegobench plan` estimates before you commit.** It takes the command you
would run, rather than its own flags, so a plan cannot describe a different
run from the one that would happen:

```sh
stegobench plan score --corpus ./pentimento-nano --detector zsteg
```

It counts the corpus rather than guessing from its size, and where a tool
declares no measured rate it says the time is unknown instead of inventing
one, which would be the estimate lying about the only thing it is for.

**Thirteen tools are registered** under `plugins/registry/` today: six
embedders (steghide, outguess, openstego, stegosuite, hstego, and Stegcore's
embed side) and seven detectors (Aletheia's SPA, RS and rich-model estimators,
StegExpose, zsteg, plus Stegcore and StegaShield as subjects rather than
references). Stegcore appears twice because it does both jobs, and hiding a
payload and judging one are different measurements that should not share an
identifier.
`stegoveritas` has never built against current dependencies here, so it is
deliberately not registered and not listed anywhere as if it worked. It stays
out until it builds, rather than sitting in the registry as a dead reference.

**stegoveritas, F5, jsteg and jphide are not present.** They're candidates
for later, not silently dropped: naming them here rather than letting a
reader discover the gap is the point of this section existing at all.

## Verifying a download

Every file attached to a release is signed, `SHA256SUMS` included. That matters
because a checksum on its own only tells you the bytes match a list, and
whoever can serve you a tarball can serve you a matching list. A signature
tells you the file came out of this repository's release workflow.

Nothing is released yet, so these commands describe what a release will carry
rather than something you can run today. Replace `v1.2.3` with the tag you
downloaded.

**The short way**, if you have GitHub's `gh` tool
([install it](https://cli.github.com/)):

```sh
gh attestation verify stegobench-v1.2.3-x86_64-unknown-linux-musl.tar.gz \
    --repo elementmerc/stegobench
```

That checks GitHub's own record of which workflow run built the file.

**The way that works from anywhere**, including a mirror or an archived copy,
using [cosign](https://docs.sigstore.dev/cosign/system_config/installation/).
Download the archive plus its `.sig` and `.pem`, and put them in one directory:

```sh
cosign verify-blob \
    --certificate stegobench-v1.2.3-x86_64-unknown-linux-musl.tar.gz.pem \
    --signature stegobench-v1.2.3-x86_64-unknown-linux-musl.tar.gz.sig \
    --certificate-identity-regexp '^https://github\.com/elementmerc/stegobench/\.github/workflows/release\.yml@refs/tags/v' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com \
    stegobench-v1.2.3-x86_64-unknown-linux-musl.tar.gz
```

It prints `Verified OK` and exits 0, or it fails. There's no key to fetch
first: the signature carries a short lived certificate naming the workflow that
made it, and cosign checks that certificate against Sigstore's public
transparency log.

**Check the checksums file the same way**, then use it. Verifying `SHA256SUMS`
and then running `sha256sum -c` covers every file in one step:

```sh
cosign verify-blob \
    --certificate SHA256SUMS.pem \
    --signature SHA256SUMS.sig \
    --certificate-identity-regexp '^https://github\.com/elementmerc/stegobench/\.github/workflows/release\.yml@refs/tags/v' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com \
    SHA256SUMS

sha256sum -c SHA256SUMS
```

The long `--certificate-identity-regexp` line is the part that matters: it says
which repository, which workflow file and which kind of ref the signature has
to come from. Drop it and cosign will happily accept a signature from anybody
at all. `SECURITY.md` says what this does and doesn't prove.

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

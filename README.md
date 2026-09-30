# Stegobench

**Stegobench measures how good a steganography detector is.** You give it
images whose answers are already known (this one is clean, this one hides a
payload), it runs a detector over every one of them, and it reports how often
the detector was right, in a versioned JSON document that names the exact
bytes the number was measured on.

**It does not examine your own images.** If your question is "is something
hidden in these pictures", that's the opposite direction: unknown images, and
a detector you already trust. Stegobench is how you find out whether to trust
one. Run `stegobench help scope` for the difference in full, and for where to
go instead.

Most published steganalysis results can't be checked by the people reading
them. The corpora usually can't be redistributed, and the discipline that
keeps a measurement honest (a clean image and its stego twin must differ in
nothing but the payload, a cover and its stego twin must land on the same
side of a train/test split) is described in a paper rather than enforced by
the tool that produced the number. Stegobench is an attempt at fixing both.

## Two halves, two names, one repository

| | What it does | Language | Install |
|---|---|---|---|
| **`stegobench`** | the harness: runs detectors over a labelled corpus and scores them | Rust | `cargo install --path crates/stegobench-cli` |
| **`pentimento`** | the generators: build, audit, pack and verify a labelled corpus in the first place | Python | `pip install -r requirements.lock && pip install -e . --no-deps` |

They're deliberately not given one name. Two different programs sharing one
name on one PATH is a worse problem than the one a single entry point would
solve.

If you only want the corpus and your own code, you don't need either:
[Pentimento](https://github.com/elementmerc/pentimento) is published on
Internet Archive, HuggingFace and Kaggle, and it's a JPEG-decompressed spatial
corpus, not comparable to BOSSbase. Read `docs/design/pentimento.md` before
quoting a number from it.

## What's here today, honestly

The two halves are at different stages:

- **A Rust command-line tool** (`stegobench`) that reads a registry of
  detectors, embedders and corpora, checks whether this machine can run them,
  and scores a corpus against them. Every command in the tree is built and
  tested: `schema`, `validate`, `verify`, `list`, `describe`, `doctor`,
  `plan`, `score`, `fetch`, `report`, `completions` and `help`.
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

## Your first five minutes

Install it, then work down the page. Every command below is one you can run,
and the output shown is output it printed.

```sh
cargo install --path crates/stegobench-cli
```

You need no configuration files, and no checkout for the tool itself: the
registry is compiled into the binary as a fallback, so `stegobench` works from
any directory on a fresh machine. Steps 4 to 6 below read files that ship in
the clone (`corpora/starter` and `results/v1`), so run them from it.

**1. What is this?** Type the bare name.

```
$ stegobench
stegobench measures how good a steganography detector is, by running it over images whose answers are already known.
It does NOT examine your own images (`stegobench help scope`).

  stegobench list detectors    what this installation can run
  stegobench doctor            what is installed, and what it needs
  stegobench help              the reasoning, one topic at a time
  stegobench --help            every command and flag

  stegobench describe pentimento-core    10,000 covers with their licences attached, to quote a number from
```

**2. What can it run?** The list is read from the registry, so it can't go
stale the way a README can.

```
$ stegobench list detectors
aletheia-rich    MIT                container  stegobench/aletheia-rich
aletheia-rs      MIT                container  stegobench/aletheia
aletheia-spa     MIT                container  stegobench/aletheia
stegashield      proprietary        container  5iprojects/stegashield  needs STEGASHIELD_LICENCE
stegcore         AGPL-3.0-or-later  local      stegcore
stegexpose       GPL-3.0            container  stegobench/stegexpose
zsteg            MIT                container  stegobench/zsteg

container  pinned by image digest, sandboxed, no network. Needs a container runtime.
local      a program you installed, pinned by the hash of the file that ran. No sandbox, your network.

SERVICE, not sandboxed, and needs the network: stegashield. The image names the subject; an adapter here reaches an instance you started. Nothing checks that instance was built from that image, so a result of it reads `unpinned`.

`stegobench doctor` says what each one still needs.

13 tools in 10 images. Up to 1055 MB bundled, 21.3 GB more on demand.

registry  plugins/registry
```

**3. Can this machine actually run them?** `doctor` checks each tool's
container or binary and prints the line to type for each one that's missing.
Its first line names which registry answered, because a machine can hold more
than one.

```
$ stegobench doctor --no-selftest
registry  plugins/registry

aletheia-rich    MISSING   not pulled. docker pull stegobench/aletheia-rich@sha256:5b08e93aaed2b2c30753ec3654df41d1406a4669f998384a04c26e24952f3ddf  not verified
    docker pull stegobench/aletheia-rich@sha256:5b08e93aaed2b2c30753ec3654df41d1406a4669f998384a04c26e24952f3ddf
        about 9090 MB, pinned by digest
...
13 tool(s): 0 verified, 0 answering, 0 broken, 12 not installed, 1 undetermined, 13 not checked.
1 undetermined: nothing to install would settle it. Its line says what it needs.
not checked is not the same as working; each line says why.
13 need something from you, indented under each.
```

That run was from a clone, so `./plugins/registry` answered. Away from one the
same line reads `registry  built in`.

Drop `--no-selftest` and it also asks each installed tool to flag a known
planted signal and clear a known clean fixture, in both directions, because a
tool that answers "stego" to everything passes a one-sided check.

**4. Check what a run would cost, before you commit to it.** `plan` takes the
command you'd type, so it can't describe a different run from the one that
would happen. It counts the corpus rather than guessing from its size, and
says the time is unknown where a tool declares no measured rate:

```
$ stegobench plan score --corpus corpora/starter --detector all
18 item(s) to score with each of 7 detector(s):
aletheia-rich    about 2 minutes
aletheia-rs      about 25 seconds
aletheia-spa     about 14 seconds
stegashield      about 16 seconds
stegcore         about 4 seconds
stegexpose       about 5 seconds
zsteg            about 2 seconds

Total          about 3 minutes, over the 7 that declare a rate
Records        about 0.0 MB
Worst case     2.1 hours (every item hitting the 60s deadline)
Configuration  custom: no --corpus-id, so nothing to check this directory against. Comparable with itself, not with anybody else's number
```

`corpora/starter` is the corpus that ships in the box: six covers and twelve
stego images, enough to watch the machinery work and far too few to quote a
number from. `corpora/starter/README.md` says so at more length.

**5. Score the corpus.** This is the job. Point it at a directory of labelled
samples and a detector you have installed:

```sh
stegobench score --corpus corpora/starter --detector zsteg --out result.json
```

It asks the detector about every image, writes each answer as it goes, and
emits a validated `result-v1` document. Interrupt it and run the same command
again and it picks up where it stopped. `--detector all` runs every registered
detector over the same bytes in one pass.

Point it at a folder of your own photographs and it refuses, and explains why
rather than reporting a missing file:

```
$ stegobench score --corpus ./holiday-photos --detector all
./holiday-photos holds 3 image(s) and no records saying which of them hides anything, so there is nothing to be right or wrong about.

Stegobench measures DETECTORS against labelled images; it does not examine your own.
`stegobench help scope`      the difference, and where to go instead
`stegobench list detectors`  what is registered here
`stegobench help pairing`    what a corpus has to carry first
```

It exits 3, a pre-flight refusal, which a script can tell apart from an error
and knows not to retry.

**6. Turn results into a table.** `report` reads result documents and renders
the conditions into every row, so a figure can't be lifted out without them.
This repository ships 21 real result documents under `results/v1`:

```
$ stegobench report results/v1 --format markdown
# Steganalysis results

21 result document(s) from results/v1, in 1 table(s).
Ordered by arm then detector, never by score. Not a ranking. (`stegobench help reports`)

## round3-q95 (custom)

Corpus digest: `sha256:481a17f2592dd6042b9298f436150ae08d0d0c70d0ccffebe647f075166d77b6`

CUSTOM: round3-q95 declared no digest in advance. Quote these figures beside nothing else.

| detector | isolation | corpus | config | arm | domain | AUC | TPR@1%FA | TPR@10%FA | pairing | split | clean/stego/unscored | conditions |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| aletheia-rs | sandbox-no-network | round3-q95 @ sha256:481a17f2 | custom | outguess at 5.000% of capacity | jpeg | 0.4909 | 0.0063 | 0.0813 | confounded | not-applicable | 158/160/2 | CONFOUNDED: the pair differs in more than the payload · 2 image(s) unscored and not counted |
| aletheia-spa | sandbox-no-network | round3-q95 @ sha256:481a17f2 | custom | outguess at 5.000% of capacity | jpeg | 0.4923 | 0.0000 | 0.0563 | confounded | not-applicable | 158/160/2 | CONFOUNDED: the pair differs in more than the payload · 2 image(s) unscored and not counted |
| stegexpose | sandbox-no-network | round3-q95 @ sha256:481a17f2 | custom | outguess at 5.000% of capacity | jpeg | 0.4813 | 0.0000 | 0.0563 | confounded | not-applicable | 160/160/0 | CONFOUNDED: the pair differs in more than the payload |
```

Eighteen further rows follow, one for each remaining detector and arm.

Rows are ordered by arm and then by detector, never by score. It isn't a
ranking and no ranking can be derived from it. Every row above is flagged
`confounded`, which is the point: these are real measurements, and the report
says what's wrong with them in the same cell as the number.

### Two things worth knowing early

Every subcommand accepts `--json`, which puts machine-readable output on
stdout and leaves progress and human text on stderr, so
`stegobench doctor --json | jq` works while you can still watch it run.

Conceptual reasoning that doesn't fit on a `--help` line lives behind
`stegobench help <topic>`: `scope` is what this measures and what it doesn't,
`pairing` is why a clean image and its stego twin must differ in nothing but
the payload, and `results` is how to judge a number somebody else produced.

### Where the registry comes from

In order: `--registry` or `STEGOBENCH_REGISTRY`, then `./plugins/registry`,
then beside the executable, then your user data directory, then the system
data directory, and finally the copy compiled into the binary. A path you name
yourself is used as given: if it isn't there that's an error, never a quiet
fall back to a different registry. `stegobench doctor` prints which one
answered.

**The first one found answers in full.** Registries aren't merged, so a
directory holding one detector of your own gives you one detector and not
thirteen. To add your own tool to the set that ships, copy `plugins/registry/`
somewhere and add your file to the copy. `stegobench list` then shows all of
them and names the directory it read.

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

A detector that answers over HTTP rather than on a command line is registered
the same way, with `host = true` and a small adapter script that does the
posting. `docs/guide/http-detector.md` walks that case from installing the tool
to a result you can quote. The address of your instance is never written into
the entry: an entry naming a loopback or private-network address is refused,
because a shipped address is scored against whatever answers on it.

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
way they read a tool, so a user sees one registry. `describe` prints a summary
written for a reader; `describe <id> --toml` prints the entry itself and
nothing else, for a script. Where an entry declares a
download route, `stegobench fetch <id> --tier <tier>` downloads it and checks
the bytes against the SHA-256 and the size the entry declared in advance. A
corpus whose terms don't permit redistribution is refused before a connection
opens, because fetching somebody else's dataset for you would make this
project the mirror; `describe` prints how to obtain those yourself.

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

## Installing the Python half

One recipe. Use this one:

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements.lock
.venv/bin/pip install -e . --no-deps
```

`requirements.lock` is every package that was installed on the machine that
built the published corpus, including `numba` and `llvmlite`. Those two
compile the `conseal` code that decides which pixels carry the payload, so a
different version of either can change the bytes a build produces. `--no-deps`
stops pip resolving round the lock it was just given.

Two shorter recipes exist and both float that compiler stack:

| Recipe | What you get |
|---|---|
| `pip install -e .` | the `pentimento` command, with `numba` and `llvmlite` resolved to whatever is newest today |
| `pip install -r requirements.txt` | the seven packages this project chose, and no `pentimento` command |

Neither is wrong; neither reproduces a published number either. A build
command run outside the locked set says so on stderr before it starts.

The lock records one machine, Python 3.14 on Linux x86_64. Elsewhere, install
`requirements.txt` and read that warning as a real caveat rather than noise:
your build will be internally consistent and will not be byte-identical to the
published one.

## Building a corpus (the Python half)

```sh
# 1. covers, with provenance and licence recorded per file
.venv/bin/pentimento fetch-commons --out covers/ --count 1000 \
    --dedup-db dedup.sqlite3 --licences permissive

# 2. assign the tier order. A tier is a prefix of one ordering over the
#    whole corpus, which no single fetch can know, so it is assigned once
#    the cover set is complete. Nothing downstream runs without it.
.venv/bin/pentimento manifest-repair covers/manifest.jsonl

# 3. an embedding arm
.venv/bin/pentimento build-adaptive-arms --covers covers/ --out arms/adaptive \
    --count 1000 --schemes suniward --rates 0.4

# 4. score it against a detector you're testing
.venv/bin/pentimento score-arms --corpus arms/adaptive \
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
pairs, built in a single 22 hour run from covers already on disk, with every
arm resumable and every file checksummed. To size a build of your own, run
`pentimento build-core-tier --dry-run`: fetching the covers is a separate
budget and the larger one.

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
records, the directory in front of the tool matches it, and every image turns
out to be the file its own record describes. That last check costs one extra
read of the corpus and is what stops somebody keeping a real manifest and
putting easier images underneath it. A directory that
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

**F5, jsteg and jphide are not present either.** They're candidates for later,
not silently dropped: naming them here rather than letting a reader discover
the gap is the point of this section existing at all.

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

## Citing this work

There are two things here and they're cited differently, so pick by what your
sentence is claiming.

**If you're citing a number, cite the corpus.** A measurement belongs to the
images it was taken on. The harness is the instrument, and naming the
instrument doesn't tell a reader which bytes produced the figure. Pentimento
lives in [its own repository](https://github.com/elementmerc/pentimento) for
exactly this reason.

```bibtex
@misc{pentimento,
  author       = {Daniel Iwugo},
  title        = {Pentimento: a matched-pair steganalysis corpus},
  howpublished = {\url{https://github.com/elementmerc/pentimento}},
  note         = {JPEG-decompressed spatial covers. Not comparable to BOSSbase.
                  State the tier and the arm alongside any figure}
}
```

Say which tier (Nano, Lite or Core) and which arm the number came from. A
figure without them isn't reproducible, and the tiers are strict prefixes of
one another, so the tier is part of what was measured.

**If you're citing the tool itself**, because you used it, extended it or are
comparing methodologies, cite Stegobench. `CITATION.cff` at the repository root
carries the machine-readable version, and GitHub renders a "Cite this
repository" button from it.

```bibtex
@misc{stegobench,
  author       = {Daniel Iwugo},
  title        = {Stegobench: a reproducible benchmark for steganalysis},
  howpublished = {\url{https://github.com/elementmerc/stegobench}}
}
```

Neither entry carries a version, a date or a DOI, and that's deliberate:
nothing has been tagged and no archive has minted an identifier yet, so every
one of those fields would be a guess. They go in when they're true.

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

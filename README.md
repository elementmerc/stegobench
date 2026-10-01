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

## What to install

**To score detectors, you need one thing: the `stegobench` binary.**

```sh
cargo install --path crates/stegobench-cli
```

No configuration files, no checkout: the registry and a starter corpus are
compiled into the binary, so it works from any directory on a fresh machine.

There's a second, separate program in this repository, `pentimento`, written
in Python. **You need it only if you're building a corpus of your own.** Most
people never will; `stegobench fetch` gets you a corpus to score without it.
`generators/README.md` covers it if you do.

| | What it does | You need it when |
|---|---|---|
| **`stegobench`** | runs detectors over a labelled corpus and scores them | always |
| **`pentimento`** | builds, audits, packs and verifies a corpus from scratch | only if you're making a corpus |

They're two names on purpose. Two different programs sharing one name on one
PATH is a worse problem than the one a single entry point would solve.

If all you want is the corpus and your own code, you need neither:
[Pentimento](https://github.com/elementmerc/pentimento) is published on
Internet Archive, HuggingFace and Kaggle. It's a JPEG-decompressed spatial
corpus and is **not comparable to BOSSbase**; read `docs/design/pentimento.md`
before quoting a number from it.

## Your first five minutes

Every command below is one you can run, and the output shown is output it
printed.

**1. What is this?** Type the bare name.

```
$ stegobench
stegobench measures how good a steganography detector is, by running it over images whose answers are already known.
It does NOT examine your own images (`stegobench help scope`).

  stegobench list detectors    what this installation can run
  stegobench doctor            what is installed, and what it needs
  stegobench help              the reasoning, one topic at a time
  stegobench --help            every command and flag
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
stegexpose       none-granted       container  stegobench/stegexpose
zsteg            MIT                container  stegobench/zsteg
```

`container` means pinned by image digest, sandboxed, no network. `local` means
a program you installed, pinned by the hash of the file that ran.

**3. Can this machine actually run them?** `doctor` checks each tool's
container or binary and prints the line to type for each one that's missing.
Drop `--no-selftest` and it also asks each installed tool to flag a known
planted signal and clear a known clean fixture, in both directions, because a
tool that answers "stego" to everything passes a one-sided check.

```sh
stegobench doctor
```

**4. Get a corpus.** The starter corpus is compiled into the binary, so this
needs no clone and no network:

```sh
stegobench fetch stegobench-starter --tier nano --out ./starter
```

Six covers and twelve stego images: enough to watch the machinery work, far
too few to quote a number from.

**5. Check what a run would cost before you commit to it.** `plan` takes the
command you'd type, so it can't describe a different run from the one that
would happen. It counts the corpus rather than guessing from its size, and
says the time is unknown where a tool declares no measured rate.

```sh
stegobench plan score --corpus ./starter --detector all
```

**6. Score it.** This is the job.

```sh
stegobench score --corpus ./starter --detector zsteg --out result.json
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
```

It exits 3, a pre-flight refusal, which a script can tell apart from an error
and knows not to retry.

**7. Turn results into a table.** `report` renders the conditions into every
row, so a figure can't be lifted out without them. Rows are ordered by arm and
then detector, never by score: it isn't a ranking and no ranking can be
derived from it.

```sh
stegobench report results/v1 --format markdown
```

This repository ships 21 real result documents under `results/v1`. Every row
is flagged `confounded`, which is the point: they're real measurements, and
the report says what's wrong with them in the same cell as the number.

## The commands

`schema`, `validate`, `verify`, `list`, `describe`, `embed`, `doctor`, `plan`,
`score`, `metrics`, `fetch`, `report`, `completions`, `help`. Every one is
built and tested. `stegobench --help` has the flags.

Every subcommand accepts `--json`, which puts machine-readable output on
stdout and leaves progress and human text on stderr, so
`stegobench doctor --json | jq` works while you can still watch it run.

Reasoning that doesn't fit on a `--help` line lives behind
`stegobench help <topic>`: `scope` is what this measures and what it doesn't,
`pairing` is why a clean image and its stego twin must differ in nothing but
the payload, and `results` is how to judge a number somebody else produced.

## What it checks that you'd otherwise have to remember

`score` enforces the two rules rather than describing them:

- A corpus that puts a cover and its stego twin on **opposite sides of a train
  and test split** stops the run, instead of producing an inflated number
  nobody could spot afterwards.
- For every stego image that names the cover it came from, the headers of both
  are compared. A difference in format, size, bit depth or channel count means
  **something other than the payload changed**, and the result says so and
  names the images. Matching headers prove nothing on their own, so the result
  distinguishes "looked and found nothing" from "could not look".

Point it at a registered corpus with `--corpus-id` and it checks that claim
rather than taking it. A run is marked `named` only when the entry declares
the digest of its records, the directory matches it, and every image turns out
to be the file its own record describes. Otherwise the run is `custom`, which
is a perfectly good run: comparable with itself rather than with somebody
else's.

**One honest limit today:** `score` reads an unpacked corpus directory, so a
packed tier has to be extracted first.

## Adding your own detector

A detector or embedder is a TOML file under `plugins/registry/`, not code
wired into the harness:

```toml
name = "my-detector"
kind = "detector"
licence = "MIT"

[image]
reference = "ghcr.io/you/my-detector@sha256:..."   # a tag is refused
size_mb = 200
bundled = true            # derived from the size, not chosen: true at or
                          # below 750 MB, and an entry whose flag disagrees
                          # with its own size is refused

[emits]
output = "score"          # or "verdict" if it only says yes or no

[accepts]
formats = ["png"]

[selftest]
must_detect = "fixtures/lsb-0.4bpp.png"   # it must flag this
must_clear = "fixtures/clean.png"          # and clear this
```

Both fixtures are required: a tool that answers "stego" to everything would
otherwise pass a one-sided check.

A tool is registered either as `[image]` (a container pinned by digest, run
with no network, identical bytes on two machines) or `[binary]` (a program you
installed, hashed on your machine), never both.

A detector that answers over HTTP is registered the same way, with
`host = true` and a small adapter script. `docs/guide/http-detector.md` walks
that case end to end. The address of your instance is never written into the
entry: an entry naming a loopback or private-network address is refused,
because a shipped address is scored against whatever answers on it.

See `stegobench help plugins` for the full reasoning, and
`docs/guide/` for registering a **corpus**, which is a different shape because
a corpus is data you point at rather than code you run.

## Thirteen tools are registered

Six embedders (steghide, outguess, openstego, stegosuite, hstego, and
Stegcore's embed side) and seven detectors (Aletheia's SPA, RS and rich-model
estimators, StegExpose, zsteg, plus Stegcore and StegaShield as subjects
rather than references). Stegcore appears twice because it does both jobs, and
hiding a payload and judging one are different measurements that shouldn't
share an identifier.

`stegoveritas` has never built against current dependencies here, so it isn't
registered. F5, jsteg and jphide aren't present either. They're candidates for
later, not silently dropped.

## Citing this work

**If you're citing a number, cite the corpus.** A measurement belongs to the
images it was taken on, and naming the instrument doesn't tell a reader which
bytes produced the figure.

```bibtex
@misc{pentimento,
  author       = {Daniel Iwugo},
  title        = {Pentimento: a matched-pair steganalysis corpus},
  howpublished = {\url{https://github.com/elementmerc/pentimento}},
  note         = {JPEG-decompressed spatial covers. Not comparable to BOSSbase.
                  State the tier and the arm alongside any figure}
}
```

Say which tier (Nano, Lite or Core) and which arm the number came from. The
tiers are strict prefixes of one another, so the tier is part of what was
measured.

**If you're citing the tool**, because you used it, extended it or are
comparing methodologies, cite Stegobench. `CITATION.cff` carries the
machine-readable version.

```bibtex
@misc{stegobench,
  author       = {Daniel Iwugo},
  title        = {Stegobench: a reproducible benchmark for steganalysis},
  howpublished = {\url{https://github.com/elementmerc/stegobench}}
}
```

Neither entry carries a version, a date or a DOI: nothing has been tagged and
no archive has minted an identifier yet, so every one of those fields would be
a guess. They go in when they're true.

## Verifying a download

Every file attached to a release is signed, `SHA256SUMS` included, because a
checksum on its own only tells you the bytes match a list, and whoever serves
you a tarball can serve you a matching list. A signature tells you the file
came out of this repository's release workflow.

Nothing is released yet, so these describe what a release will carry. Replace
`v1.2.3` with the tag you downloaded.

**With GitHub's `gh` tool**, which checks GitHub's own record of which
workflow run built the file:

```sh
gh attestation verify stegobench-v1.2.3-x86_64-unknown-linux-musl.tar.gz \
    --repo elementmerc/stegobench
```

**From anywhere**, including a mirror or an archived copy, with
[cosign](https://docs.sigstore.dev/cosign/system_config/installation/):

```sh
cosign verify-blob \
    --certificate SHA256SUMS.pem \
    --signature SHA256SUMS.sig \
    --certificate-identity-regexp '^https://github\.com/elementmerc/stegobench/\.github/workflows/release\.yml@refs/tags/v' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com \
    SHA256SUMS

sha256sum -c SHA256SUMS
```

Verifying `SHA256SUMS` and then running `sha256sum -c` covers every file in
one step. There's no key to fetch first: the signature carries a short-lived
certificate naming the workflow that made it.

The long `--certificate-identity-regexp` line is the part that matters. It
says which repository, which workflow file and which kind of ref the signature
has to come from. Drop it and cosign will accept a signature from anybody at
all. `SECURITY.md` says what this does and doesn't prove.

## Running the test suites

```sh
cargo test --workspace
.venv/bin/python -m unittest discover -s generators -p "test_*.py"
.venv/bin/python -m unittest discover -s tools/release -p "test_*.py"
```

These are the suites CI runs, so a green result here is a green result there.

## Licence

AGPL-3.0-or-later. See `LICENSE`.

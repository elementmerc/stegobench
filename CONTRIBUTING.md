# Contributing to Stegobench

Thanks for looking. This file gathers what's already true about working in
this repository: how to add a tool, how to register a corpus, how to run the
tests, and which branch your work belongs on. It doesn't invent process that
nobody has followed yet.

Read the README's "What's here today, honestly" section first, and treat it as
the current statement of what works. The short version: every command in the tree is
built and tested. `score` reads an unpacked corpus directory rather than a
packed tier, and marks such runs `custom`, and both limits are stated in the
README rather than left to be discovered. The commands were named before they
were written so that the vocabulary would settle before anything started
depending on it.

## Building it

```sh
cargo build --release -p stegobench-cli
./target/release/stegobench --help
```

The Python half installs separately, and the two deliberately have different
names because they're two different programs:

```sh
python3 -m venv .venv
.venv/bin/pip install -e .
.venv/bin/pentimento --help
```

## Branches

`dev` and `main`, and nothing else. Routine work lands on `dev`. `main` moves
forward only for a release, as a fast forward from `dev`. Please open pull
requests against `dev`.

## Adding a detector or an embedder

A detector or an embedder is a TOML file under `plugins/registry/`, not code
wired into the harness. You don't need to touch Rust to add one.

- Detectors go in `plugins/registry/detectors/`.
- Embedders go in `plugins/registry/embedders/`.

Copy an existing entry as a starting point. `plugins/registry/embedders/steghide.toml`
is a short one, and `plugins/registry/detectors/aletheia-rs.toml` shows an
entry that needs an adapter script.

A minimal detector entry looks like this:

```toml
name = "my-detector"
kind = "detector"
licence = "MIT"

[image]
reference = "ghcr.io/you/my-detector@sha256:..."   # a tag is refused
size_mb = 200
bundled = true            # derived from size_mb, not a free choice

[emits]
output = "score"          # or "verdict" if it only says yes or no

[accepts]
formats = ["png"]

[selftest]
must_detect = "fixtures/lsb-0.4bpp.png"
must_clear = "fixtures/clean.png"
```

Four rules the entry is held to, and each one comes from something that went
wrong:

**A container is pinned by digest, or the entry is refused.** The registry
rejects `[image] reference` written as a tag, because a tag can be repointed
after a measurement was taken and then nobody can say which bytes produced the
number. A locally installed binary is the other accepted shape, declared in a
`[binary]` block with the argv that invokes it and the arguments that make it
print its version, so a run records the build it actually called rather than
the one that was installed when the entry was written. An entry declares an
image or a binary, never both.

**`bundled` is derived, not chosen.** An image of 750 MB or less is bundled
and anything larger isn't, and the registry refuses an entry whose flag
disagrees with its `size_mb`. It's a check rather than a setting so that the
default image's contents can't drift away from the sizes that justify them.

**Both self-test fixtures are required.** A tool that answers "stego" to
everything, or "clean" to everything, passes a one-sided check. `stegobench
doctor` runs `must_detect` and `must_clear` before it believes a tool works,
and an entry that declares no `[selftest]` at all is refused. An embedder
declares a `[roundtrip]` table as well, because the question there is
different: hide a known payload, extract it, and compare the bytes. See
`plugins/registry/embedders/steghide.toml` for both tables together.

**The entry describes cost honestly if it declares cost at all.** `[cost]`
records seconds per image, peak memory and cores per worker. Nothing scores a
run against those numbers yet, which is exactly why a guess there is easy to
get away with and expensive later: the estimate a long run gets planned
against will read them. Leave the table out rather than filling it with
figures you haven't measured.

Run `stegobench help plugins` for the full reasoning behind the registry
design, and `stegobench describe <name>` to see how your entry reads back.

## Registering a corpus

A corpus is a TOML file under `plugins/registry/corpora/`, and it's a
different shape from a plugin for a plain reason: a corpus is data you point
at, and a plugin is code you run. It has a licence, a download route and a
cover count; it has no image, no argv and no self test.

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

`status = "verified"` needs all four of an identifier, a link to the licence
text, a date and a note saying what was read. Anything short of that is
`unverified` or `none-granted`, and naming a licence beside either of those is
refused. Mirrors of well-known corpora carry MIT, Apache 2.0 and CC0 where the
original granted none of them, and a guess written into a metadata field is
how that starts.

`redistribution` is a separate field from the licence because it's a separate
question. A corpus you may use isn't always one you may publish.

## Running the tests

Both suites, exactly as CI runs them:

```sh
cargo test --workspace

python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
.venv/bin/python -m unittest discover -s generators -p "test_*.py"
.venv/bin/python -m unittest discover -s tools/release -p "test_*.py"
```

Python tests that need something from `requirements-optional.txt` skip rather
than fail when it isn't installed.

Before you push, the same checks CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

If you added a Python module under `generators/`, `test_readme.py` fails until
`generators/README.md` has a row for it. That's deliberate: the table is the
index, and an undocumented module is an invisible one.

## Writing style in documentation

The prose here is British English, uses contractions, and avoids em dashes as
a dramatic pause. Commit hooks on the maintainer's side check the first and
the last of those, so a pull request that trips one gets a note asking for a
wording change rather than a rejection. Explain why a thing is the way it is;
the code already says what it does.

## Reporting a problem

Open an issue with the templates under `.github/ISSUE_TEMPLATE/`. For a bug,
the output of `stegobench doctor --json` is the single most useful thing you
can attach: it says which tools this machine can actually run and what it
found when it asked them.

For anything that looks like a security problem, don't open an issue. See
`SECURITY.md`.

## Licence

Contributions are accepted under AGPL-3.0-or-later, the licence both halves of
this repository ship under. By opening a pull request you're agreeing that
your contribution can be distributed under it.

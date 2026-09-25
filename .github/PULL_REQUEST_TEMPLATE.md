<!--
Thanks for the pull request. Please open it against `dev`; `main` only moves
at a release.

Delete whatever doesn't apply. A one line pull request doesn't need a long
description, and a registry entry needs almost none of this.
-->

## What this changes, and why

<!-- The why matters more than the what. The diff already says what. -->

## How it was checked

<!--
Which of these you ran, and what happened. Say "not run" where that's the
truth; an unrun check reported as green is worse than no check.

    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    .venv/bin/python -m unittest discover -s generators -p "test_*.py"
    .venv/bin/python -m unittest discover -s tools/release -p "test_*.py"
-->

## If this adds or changes a registry entry

- [ ] The container is pinned by digest, or it's a `[binary]` entry with its
      argv and version arguments. Never a mutable tag.
- [ ] The `[selftest]` table declares both directions: a fixture the tool must
      flag and one it must clear. An embedder adds a `[roundtrip]` table on top
      of that.
- [ ] `stegobench list` and `stegobench describe <name>` read the entry back
      without complaint.
- [ ] Any `[cost]` figures were measured rather than guessed, or the table was
      left out.

## If this changes documentation

- [ ] British English, contractions, no em dashes used as a dramatic pause.
- [ ] Nothing here claims a feature works that doesn't. The README's "What's
      here today, honestly" section is the standard to match.

## Anything a reviewer should look at twice

<!-- Known gaps, a decision you weren't sure about, a test you couldn't run. -->

// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Conceptual material that does not belong on a flag.
//!
//! Written for a reader who never opens the repository, so each topic repeats
//! the reasoning already carried in the code and in `generators/README.md`
//! rather than pointing at it.

pub const TOPICS: &[&str] = &["pairing", "splits", "licences", "plugins"];

pub fn text(topic: &str) -> Option<&'static str> {
    match topic {
        "pairing" => Some(PAIRING),
        "splits" => Some(SPLITS),
        "licences" => Some(LICENCES),
        "plugins" => Some(PLUGINS),
        _ => None,
    }
}

const PAIRING: &str = "\
pairing: why a clean image and its stego twin must differ in nothing else

Every corpus arm writes its clean cover and its stego image from the same \
source array, through the same code path, so the two differ ONLY in the \
embedded payload. Where a resize is involved, the cover is resized first and \
the payload embedded afterwards, so resampling artefacts are identical on \
both sides.

This is the whole reliability of the method, not a detail. If a clean image \
and its stego twin differ in anything else, a detector can score well by \
noticing that other difference, and the result says nothing about \
steganography. It has happened here: a whole measurement round was voided \
when outguess re-encoded its output at JPEG quality 75 regardless of the \
input, while the clean half of the pair had been written at quality 95. The \
detector was not finding hidden data, it was finding a quality difference.

A `result-v1` document declares its pairing as one of:
  single-variable   clean and stego differ only in the payload (the goal)
  confounded        they differ in something else too, kept as a demonstration

`stegobench` cannot always prove pairing holds; where it can check (matching \
dimensions, matching format, matching quantisation tables where applicable) \
it does, and refuses a build that fails the check rather than shipping a \
corpus that would quietly flatter every detector run against it.
";

const SPLITS: &str = "\
splits: why a cover and its stego twin must land on the same side

`stegobench` requires by-cover train/test splitting: a cover image and every \
stego image made from it belong to the same split, never split across train \
and test.

Splitting by FILE instead of by cover lets a detector see near-duplicate \
bytes on both sides of the boundary. A model trained on a cover's stego twin \
would then be tested on the same cover's clean version, or a very close \
relative of it, and it can succeed by recognising the cover rather than by \
detecting the hidden data. That inflates every number the split touches and \
the inflation is invisible in the output: nothing about a suspiciously high \
AUC announces that it came from a leak.

A `result-v1` document's `declarations.split_discipline` records which \
applied:
  by-cover        a cover and all its stego twins share one side of the split
  by-file         no such guarantee was enforced (a known weaker claim)
  not-applicable  no train/test split applies, as for an untrained detector

The corpus manifest carries a `split_salt` per tier specifically so this \
assignment is deterministic and auditable rather than re-rolled per run.
";

const LICENCES: &str = "\
licences: why every cover carries its own, not the collection's

`manifest-v1` requires `licence` and `licence_url` on every row, not once for \
the collection. This is not caution for its own sake: a real corpus built \
here has covers under several different licences at once, over half of them \
Creative Commons Attribution, which carries an attribution obligation that a \
single collection-level licence statement would erase. A downstream user who \
redistributes the collection under one blanket licence would be breaking the \
terms of every CC BY cover in it without knowing it.

Each manifest row also carries `attribution` (who to credit) and \
`source_url` (where the file came from), so a publisher can generate a \
correct credit list mechanically instead of by hand. `stegobench validate` \
refuses a manifest row that is missing a licence or a licence URL, and \
`stegobench describe <corpus>` prints the licence breakdown so a reader does \
not have to open the manifest to find out what they are agreeing to.

None of this is a substitute for reading the actual terms of a specific \
licence before redistributing anything; it is what stops the tool itself \
from losing track of which terms apply to which file.
";

const PLUGINS: &str = "\
plugins: why a detector or embedder is a TOML file, not code

Every tool `stegobench` drives, from `zsteg` to Stegcore itself, is \
registered as a declarative entry under `plugins/registry/`, not as a \
special case wired into the harness. This is a deliberate bet: a benchmark \
that makes 'contribute a detector' mean 'edit our internals' gets very few \
contributions, and one where it means 'add a config file' gets many. \
lm-eval-harness won a large share of LLM evaluation on exactly this trade.

A registry entry is one of two shapes:
  image    a container, pinned by DIGEST, never by a mutable tag
  binary   an executable already on the machine, pinned by the SHA-256 of the
           exact file invoked, which is stronger than a version number

Both are pinned, because a tag or a version string can move under you and a \
hash cannot. `stegobench` refuses to load a registry entry whose image is \
named by tag rather than digest, so an unreproducible tool cannot enter the \
registry by accident.

Every entry also declares a self-test: a fixture it must flag and a fixture \
it must clear. A tool that is merely PRESENT is not a tool that WORKS, and \
this project has hit the failure mode of a check that could not fail more \
than once, including an extraction that ran over 2,000 images, exited zero \
every time, and produced nothing because a support package was silently \
missing. `stegobench doctor` runs both directions of the self-test before \
anybody files a bug about a result that was never really produced.

Stegcore itself sits in the registry as a detector like any other, reached \
through the same protocol as `zsteg`, because a benchmark judging a product \
built out of that product's own code is not a benchmark anyone should trust.
";

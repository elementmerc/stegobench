// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Conceptual material that does not belong on a flag.
//!
//! Written for a reader who never opens the repository, so each topic repeats
//! the reasoning already carried in the code and in `generators/README.md`
//! rather than pointing at it.

pub const TOPICS: &[&str] = &[
    "pairing", "splits", "licences", "plugins", "results", "reports",
];

pub fn text(topic: &str) -> Option<&'static str> {
    match topic {
        "pairing" => Some(PAIRING),
        "reports" => Some(REPORTS),
        "splits" => Some(SPLITS),
        "licences" => Some(LICENCES),
        "plugins" => Some(PLUGINS),
        "results" => Some(RESULTS),
        _ => None,
    }
}

const REPORTS: &str = "\
reports: putting a number on a page without leaving its conditions behind

  stegobench report results/v1
  stegobench report results/v1 --format markdown --out results.md

An AUC on its own is not a measurement. It is a measurement once you also \
know which corpus it came from, whether that corpus was the registered tier \
or a directory somebody assembled, whether the clean and stego images \
differed in anything besides the payload, and which side of the train and \
test split the pairs landed on. Change any one of those and the same number \
means something different.

That is exactly what goes missing when a person opens a result document, \
copies the figure into a table, and moves on. The number survives the trip to \
the page and the four conditions do not.

So every row this writes carries them in the row. Not in a legend, not under \
the table, not in a heading: in the cells, so a line somebody copies out of \
the middle of the table takes the caveats with it whether they meant to or \
not.

WHAT IT REFUSES TO DO

  No table across corpora.  An AUC on one corpus and an AUC on another are
                            measurements of two populations, not two scores
                            on one scale. Each corpus gets its own table,
                            named, with its digest above it. Two documents
                            naming one corpus with different digests are two
                            corpora here, because the digest names the bytes
                            and the name is a label somebody chose.
  No mixing custom in.      A `custom` run is comparable with itself and
                            nothing else, so it sits in its own section with
                            a sentence saying so, never interleaved with a
                            `named` row somebody could read it beside.
  No ranking.               Rows are ordered by arm and then by detector,
                            never by score, so the table cannot be read as a
                            league it was never entitled to be.
  No quiet gaps.            A file that is not a valid result-v1 document is
                            named at the TOP of the report with the reason,
                            and the command exits non-zero. A short table
                            that looks complete is the worst thing this
                            could produce.

THE THREE FORMATS

  text       aligned columns for a terminal. The default.
  markdown   a table to paste into an evaluation document.
  csv        every recorded field, one column each, including the full
             digests and the source path of each document. A skipped file
             appears as a row of its own with record_type=skipped_file, so a
             script reading the CSV cannot miss that the table is short.

`--out FILE` writes by rename-on-close, so a reader who opens that path sees \
either the previous file or the whole new one, never half a table. Without \
it the report goes to stdout.

WHAT THE EXIT CODE MEANS HERE

  0   every file found became a row.
  1   a file could not be read. The table printed and is short by that much.
  2   nothing under the paths given is a result document, so there was no
      honest table to print. An empty table under an exit code of zero reads
      as \"checked, nothing to worry about\", which is a different claim from
      \"nothing was found\".
  3   more documents than this will put in one report, or a directory tree
      deeper than it will walk. A refusal, with the cap named.
  6   a file found is not a valid result-v1 document. Same as 1, except the
      file was read and judged rather than unreadable.
";

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
  single-variable   no second variable was found (the goal)
  confounded        they differ in something else too, kept as a demonstration
  unverified        nothing could be compared, so neither is claimed

The positive claim cannot be proved after the fact. Proving it would mean \
decoding both images and comparing every pixel, and even that would miss a \
cover re-encoded before the payload went in. What CAN be proved is the \
refutation, and `stegobench score` does it: for every stego image that names \
the cover it came from, it reads both headers and compares format, width, \
height, bit depth and channel count. Any disagreement means something other \
than the payload changed, and the run is marked confounded and says which \
images and how they differ.

Read single-variable as \"looked and found nothing\", which is why the third \
value exists. A corpus whose stego images name no cover, or whose images this \
cannot read, comes back unverified rather than quietly reading as the good \
case: a benchmark that claims a rule held after checking nothing is the exact \
failure this tool was built to stop.

A confounded corpus is still scored, which a split violation is not. A second \
variable is a real property of some arms and is kept on purpose to show what \
it does, so the run happens and the result carries the fact. A cover leaking \
across the train and test boundary makes the number wrong while it still \
looks right, so that one refuses.
";

const RESULTS: &str = "\
results: how to judge a number somebody else produced

Somebody sends you a result-v1 document with an AUC of 0.94 in it. Every field \
below was written by this harness from what actually happened rather than by \
the person who ran it, so none of it has to be taken on trust. Check them in \
this order, which is the order of what it costs to be wrong.

1. WAS IT MEASURED ON THE CORPUS IT NAMES?

  stegobench verify their-result.json --corpus ./the-corpus

`corpus.digest` names the bytes the number came from. That recomputes it from \
a corpus on your own disk and exits 5 if the two are not about each other. A \
match proves the document and your copy describe the same records. It does not \
prove the images match their records; that means rehashing every file and \
belongs to whoever packed the release.

2. IS IT COMPARABLE TO ANYBODY ELSE'S NUMBER?

  declarations.configuration
    named     the corpus entry declared a digest in advance and this run
              matched it. It can sit in a table beside another named run
    custom    comparable with itself. An unregistered directory, or a run
              that scored part of a corpus

`custom` is not a warning, it is most runs. What it rules out is quoting the \
figure as a tier number.

3. COULD IT BE MEASURING SOMETHING OTHER THAN STEGANOGRAPHY?

  declarations.pairing
    single-variable  every pair that could be compared matched on format,
                     size, bit depth and channels. Read it as looked and
                     found nothing, not as proof
    confounded       at least one pair differs in something else, and the
                     detector is partly measuring that
    unverified       nothing could be compared, so nothing is claimed

  declarations.split_discipline
    by-cover         a cover and its stego twin stayed on one side
    not-applicable   correct for an untrained detector

A run whose corpus violated the split does not exist: the harness refuses \
rather than producing the inflated number.

4. WHO PRODUCED IT?

`declarations.self_reported` is set by the submission path, never by the \
submitter. `declarations.trained_on` names the corpus a trained detector saw, \
and a detector scored on what it trained on is not being measured.

5. WOULD YOU GET THE SAME NUMBER?

`provenance.plugins[].route` decides how far it travels. A container digest \
names bytes you can pull, so running the same command runs identical code. A \
local binary's hash names a file on their machine, and two people who both \
built from source get different hashes for the same version.

`provenance.host` records the operating system and architecture, \
`network_reachable` whether the tool could phone home, and `determinism` \
whether two runs of that tool agree at all.

THE SHORT VERSION

A number is defensible when the digest checks out, pairing is \
single-variable, the split is by-cover or genuinely not applicable, and \
trained_on is absent or names something other than what it was scored on. \
Everything else is a reason to ask one more question rather than to throw the \
number away.
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
`stegobench describe <corpus>` prints the terms a corpus is registered under, \
including whether anybody has verified them and whether republication is \
permitted, so a reader can find out what they are agreeing to before they \
download anything.

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

WHY BOTH, AND WHAT EACH COSTS YOU

Neither shape could go. Without containers a tool would run as you, with your \
network and your files, and two people could never run identical bytes. \
Without local binaries every contributor would have to publish a container \
before they could register a tool, and a commercial or platform specific \
program could not be measured at all.

  container   sandboxed, no network, identical bytes on every machine.
              Costs you a container runtime, and on macOS or Windows that
              runtime is a Linux virtual machine.
  local       native speed, nothing to install but the tool itself. Costs
              you the sandbox, and the hash pins the file on YOUR machine:
              two people who both built from source get different hashes for
              the same version and neither is wrong.

You never choose between them when you RUN something. You name a detector and \
`stegobench` uses whatever route its entry declares; `list` prints which, and \
a result records it so a reader knows whether the number travels.

A binary entry may also declare `platforms`. A tool that only exists on \
Windows is not broken on a Mac, and `doctor` says it cannot run here rather \
than reporting it missing and sending you after a package that does not exist \
for you. Leaving the field out means nobody has said, which is not a claim \
that it runs everywhere.

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

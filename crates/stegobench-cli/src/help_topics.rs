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
    "scope", "pairing", "splits", "licences", "plugins", "results", "reports",
];

pub fn text(topic: &str) -> Option<&'static str> {
    match topic {
        "scope" => Some(SCOPE),
        "pairing" => Some(PAIRING),
        "reports" => Some(REPORTS),
        "splits" => Some(SPLITS),
        "licences" => Some(LICENCES),
        "plugins" => Some(PLUGINS),
        "results" => Some(RESULTS),
        _ => None,
    }
}

const SCOPE: &str = "\
scope: what this measures, and what it does not

There are two different questions, and most people arrive with the second one.

  1. HOW GOOD IS THIS DETECTOR?
     You have images whose answers are already known: this one is clean, this
     one hides a payload, and somebody recorded which is which. You run a
     detector over all of them and count how often it was right. That is what
     stegobench does.

  2. IS SOMETHING HIDDEN IN THESE PICTURES?
     You have images nobody has labelled, and you want a verdict on them. That
     needs a detector you already trust, pointed at your own files. Stegobench
     is not that tool. It's how you find out whether to trust one.

They run in opposite directions. Question 1 starts from known answers and ends
with a judgement about the tool. Question 2 starts from a tool you believe and
ends with a judgement about the images.

IF YOU ARRIVED WITH QUESTION 2

  stegobench list detectors      names every detector registered here
  stegobench describe <name>     says where that one lives, what it costs, and
                                 the command that runs it

Run one of those directly on your own images. `describe` prints the exact
command, including the container invocation where the detector is an image
rather than a program you installed, so it can be pasted rather than
reconstructed. Then read that tool's own documentation for what its output
means, because stegobench is not in that loop and cannot vouch for a number
it did not produce.

EXPECT THE OUTPUT TO LOOK ALARMING, BECAUSE IT WILL

A detector run raw over one photograph typically prints pages of candidate
hits with confident-sounding names: key blocks, archives, text fragments.
Nearly all of them are noise. These tools are built to be run over a corpus
and thresholded, not to answer yes or no about one file, and the raw output
is the evidence before anybody has decided what counts.

So be careful with the answer you get. A detector that is right nine times in
ten still calls one clean image in ten a hit, and on a folder of a thousand
holiday photos that's a hundred wrong alarms. Which is exactly why question 1
exists, and why a number with its conditions attached is worth more than a
verdict without them.

The honest short answer to \"is there something hidden in this photo\" is that
no tool here can tell you, and any tool that says it can is overclaiming. What
you can find out is how often a given detector is right on images whose
answers are known, and that is what the rest of this program does.

WHY THERE IS NO COMMAND FOR QUESTION 2

Because a benchmark that also hands out verdicts would be grading its own
homework. The thing that makes a measurement here worth quoting is that the
answers were fixed before the detector saw the images, by somebody other than
the detector, and recorded beside each file. Unlabelled images have none of
that, so there is nothing to be right or wrong about.

  stegobench help results        what a result document carries and why
  stegobench help pairing        why a clean image and its stego twin have to
                                 differ in nothing but the payload
";

const REPORTS: &str = "\
reports: putting a number on a page without leaving its conditions behind

  stegobench report results/v1
  stegobench report results/v1 --format markdown --out results.md

AUC is area under the curve: one number from 0.5 to 1 for how well a \
detector's scores separate stego images from clean ones. 0.5 is guessing \
and 1 is perfect.\n\
\n\
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

THE COLUMNS, IN PLAIN WORDS

  AUC           Area under the curve. 0.5 is guessing, 1 is perfect. The
                bracketed pair beside it is the 95% confidence interval:
                the range the true value is probably in, given how many
                images were scored. A wide interval means too few images
                to be sure, NOT that the detector is unstable. Two AUCs
                whose intervals overlap heavily have not been shown to
                differ.
  TPR@1%FA      Of the stego images, the share this detector caught while
                raising a false alarm on 1% of the clean ones. TPR is the
                true-positive rate, FA is false alarms. This is usually
                the number a practitioner acts on, because it says what
                you catch at a false-alarm rate you can live with.
  arm           One way of hiding something, at one strength. `lsb-0400`
                is the LSB method carrying a 0.4 payload.
  domain        Where the payload sits: `spatial` is in the pixels, `jpeg`
                is in the compressed coefficients. A detector built for
                one is often useless on the other.
  pairing       Whether each stego image differs from its cover in nothing
                but the payload. See `stegobench help pairing`.
  split         Whether a cover and its stego twin stayed on the same side
                of the train and test boundary. See
                `stegobench help splits`.
  isolation     What the detector could reach while it ran.
                `sandbox-no-network` is a container with no network; `host`
                is a program on the operator's machine, with their network.
  pinned_by     Whether the digest beside the tool names bytes you could
                pull and re-run, or only a file on somebody else's machine.

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

PER ARM

An arm is one way of hiding something, at one strength. `lsb-0400` means the LSB method carrying a 0.4 payload; `wow-0200` means the WOW method at 0.2. A corpus holds several because a detector that finds a loud payload easily may find a quiet one not at all, and those are two results.

A corpus of several arms gets a second table, one row per arm, under the main one. The headline AUC pools the arms, and a pooled figure describes none of them: chance on one arm beside detection on another averages to something in between that nothing measured. Scoring `stegexpose` against the starter corpus gives a pooled 0.5972 that is really `lsb-0100` at 0.5000, exactly chance, beside `lsb-0400` at 0.6944. Another detector on the same corpus gives different figures, so read your own rather than these.

Every arm is scored against the WHOLE clean set rather than a share of it,
because a clean image belongs to no arm. Rows are sorted by arm name and
never by score, and each carries its own detector and corpus so a line
lifted out of the middle takes its conditions with it.

A corpus of one arm gets no second table, since a breakdown of one row is
the headline printed twice.

EXIT CODES

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

Somebody sends you a result-v1 document with an AUC of 0.94 in it. Every \
field below was written by this harness from what actually happened rather \
than by the person who ran it, so none of it has to be taken on trust. Check \
them in this order, which is the order of what it costs to be wrong.

WHAT THE NUMBERS ARE

AUC is area under the curve: one number from 0.5 to 1 for how well a \
detector's scores separate stego images from clean ones. 0.5 is guessing \
and 1 is perfect. Below 0.5 is not better than nothing, it usually means \
the scores run the wrong way round.

The bracketed pair, as in `AUC 0.5972 [0.2870, 0.9074]`, is the 95% \
confidence interval. It is the range the true value is probably in, given \
how many images were scored, and it is the difference between a number you \
can quote and one you cannot. Eighteen images gives an interval so wide it \
covers nearly everything, which is the honest report of eighteen images. \
A wide interval does NOT mean the detector is unstable; it means too few \
images were scored to say much. Two detectors whose intervals overlap \
heavily have not been shown to differ, however far apart their AUCs look.

`0.5000 [0.5000, 0.5000]` is a special case worth recognising. An interval \
of zero width means every answer was identical, so the AUC is 0.5 by \
construction rather than by measurement: the detector did not separate \
anything. `score` says so out loud when it happens.

1. WAS IT MEASURED ON THE CORPUS IT NAMES?

  stegobench verify their-result.json --corpus ./the-corpus

`corpus.digest` names the bytes the number came from. That recomputes it from \
a corpus on your own disk and exits 5 if the two are not about each other. A \
match proves the document and your copy describe the same records, and by \
default it then re-reads every image and checks it against the digest its own \
record states. That second check is the one that matters: the corpus digest \
is computed from what the records SAY, so a stego image can be swapped for an \
easier one, its record left alone, and every digest still agree. --shallow \
skips it for a corpus too large to re-read, and then says so rather than \
claiming more than it checked.

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

`provenance.plugins[].pinned_by` decides how far it travels. `image-digest` \
names bytes you can pull, so running the same command runs identical code. \
`executable-hash` names a file on their machine, and two people who both \
built from source get different hashes for the same version. `unpinned` says \
nothing in the document ties the number to particular bytes.

`provenance.plugins[].isolation` is a different question: what the tool could \
reach. `sandbox-no-network` is a container that saw nothing but the images, \
`host` is a program with their machine's network, `remote-service` means the \
images went to an instance over the wire, and `unstated` means nobody said.

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

SCORING ONE HALF

  stegobench score --corpus <name> --detector <name> --split test

A detector that learned from part of this corpus can only be measured on the \
part it never saw. `--split test` keeps the samples the corpus labels `test` \
and says how many it left out, and the document records `corpus.split` so a \
reader can tell a held-out figure from a whole-corpus one. Nothing in the \
number itself says which it is.

Without the flag a run covers both halves, which is harmless for a detector \
that learned nothing and makes the figure unquotable for one that did. \
Records are kept per half, so an interrupted half resumes into itself.

A corpus whose records state no split is refused rather than given an \
invented one, and so is a half that turns out to hold only clean or only \
stego images.
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

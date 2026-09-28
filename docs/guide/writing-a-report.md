# Writing a report somebody else can trust

You've got a folder of `result-v1` documents and you need a table for a paper,
a threat assessment, or a note to a colleague. `stegobench report` builds it.

```sh
stegobench report results/v1
stegobench report results/v1 --format markdown --out results.md
```

## What the command is actually for

An AUC on its own isn't a measurement. It becomes one once you also know four
things: which corpus it came from, whether that corpus was a registered tier
or a directory somebody assembled, whether the clean and stego images differed
in anything besides the payload, and which side of the train and test split
the pairs landed on. Change any one of them and the same number means
something different.

Those four go missing the moment a person opens a result file, copies the
figure into a table, and moves on. The number survives the trip to the page;
the conditions don't.

So every row this command writes carries them in the row. Not in a legend, not
underneath the table, not in the heading above it. In the cells, so a line
copied out of the middle of a table takes its caveats along whether the person
copying meant it or not.

## What a row says

```
detector     isolation           corpus                           config  arm                  AUC     pairing          split
aletheia-rs  sandbox-no-network  rich-suniward @ sha256:d93e9720  custom  suniward at 0.4 bpp  0.5087  single-variable  not-applicable
```

| Cell | The question it answers |
|---|---|
| `detector` and `isolation` | What was asked, and what it could reach while it answered. `sandbox-no-network` is a container that saw nothing but the images; `host` is a program on the operator's machine with their network; `remote-service` means the images went over the wire to an instance they started. How the tool is pinned is a separate question, and `--format csv` and `--format json` carry it as `pinned_by` |
| `corpus` | Which images, by name and by the first eight characters of the digest. The full digest sits above the table, and `--format csv` carries it in every row |
| `config` | `named` if the corpus digest was declared in advance and this run matched it, `custom` otherwise |
| `arm` | What was hidden and how much of it, with the unit. `0.4 bpp` and `5% of capacity` are different quantities |
| `AUC`, `TPR@1%FA`, `TPR@10%FA` | The numbers. A point the run didn't report says `not reported` rather than showing a zero |
| `pairing` | `single-variable`, `confounded` or `unverified` |
| `split` | `by-cover`, `by-file` or `not-applicable` |
| `clean/stego/unscored` | How many images each side, and how many the detector couldn't answer about |
| `conditions` | Everything above that needs a sentence: a confounded arm, a contaminated detector, a corpus with no digest |

The `conditions` cell spells its warnings out in words. `CONFOUNDED: the clean
and stego images differ in something besides the payload` is longer than a
symbol and a footnote, and it's longer on purpose: a symbol needs a legend,
and a legend is the thing a copied row leaves behind.

## The four things it won't do

**It won't put two corpora in one table.** An AUC on one corpus and an AUC on
another are measurements of two populations, not two scores on one scale. Each
corpus gets its own table with its digest above it. Two documents naming the
same corpus with different digests count as two corpora here, because the
digest names the bytes and the name is only a label somebody chose.

**It won't mix `custom` rows in with `named` ones.** A `custom` run is a
perfectly good measurement that's comparable with itself and nothing else, so
it sits in its own section with a sentence saying exactly that.

**It won't rank anything.** Rows are ordered by arm and then by detector,
never by score. A table ordered by score reads as a league whatever the prose
around it says, and `docs/leaderboard.md` is where the rules for a real
ranking live.

**It won't hide a file it couldn't read.** A file that isn't a valid
`result-v1` document is named at the top of the report with the reason, and
the command exits non-zero. The table still prints, because a table with a
named gap beats no table; the exit code is how a script knows it's short.

## Formats

| `--format` | What it's for |
|---|---|
| `text` | Aligned columns for a terminal. The default |
| `markdown` | A table to paste into a document |
| `csv` | Every recorded field, one column each, with full digests and the source path of each document |

The CSV starts each line with a `record_type` column. A file that couldn't be
read appears as a row with `record_type=skipped_file`, so a script reading the
CSV can't miss that the table is short.

The default doesn't change when you redirect stdout. A command that prints one
thing on a laptop and another in CI is a command whose output you can't
diff, and two runs over the same input here produce identical bytes.

## Writing to a file

```sh
stegobench report results/v1 --format markdown --out results.md
```

The file is written by rename-on-close, so somebody who opens that path sees
either the previous version or the whole new one. They never see half a table,
which is the one artefact this command must never produce: half a table looks
exactly like a whole one.

## Exit codes

| Code | What happened |
|---|---|
| 0 | Every file found became a row |
| 1 | A file couldn't be read. The table printed and is short by that much |
| 2 | Nothing under the paths given is a result document. There was no honest table to print |
| 3 | More documents than one report will hold, or a directory tree deeper than it walks. The cap is named in the message |
| 6 | A file found isn't a valid `result-v1` document |

Code 2 is worth a sentence. An empty table under an exit code of zero reads as
"checked, nothing to worry about", which is a different claim from "nothing was
found". So an empty result set is a refusal with the paths named, not a blank
page.

## Limits, stated

A single result document is read up to 1 MiB. The documents this project
publishes are under two kilobytes; the cap is there so a report over a
directory doesn't load whatever else is sitting in it.

One report covers up to 10,000 documents and walks up to 8 directories deep.
Past either, it refuses and says so rather than stopping quietly at the cap.

Symbolic links aren't followed during a directory walk, so a link pointing
back up the tree can't turn a report into an endless walk. A link that looks
like a result (anything ending `.json`) is listed at the top of the report
with that reason, because a row the walk couldn't deliver is still a gap.
Naming the link on the command line reads it.

## See also

- [Reading a result](/guide/reading-a-result) for judging a single document
- [Pairing, and what breaks it](/guide/pairing) for what `confounded` costs you
- [Submitting a result](/leaderboard) for the rules a published table runs under
- `stegobench help reports` for the same material at the terminal

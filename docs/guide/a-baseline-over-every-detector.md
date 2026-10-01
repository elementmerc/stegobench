# A baseline over every detector

One command, one corpus, every detector you have.

```
stegobench score --corpus ./pentimento-nano --detector all
stegobench report ./pentimento-nano.results --format markdown
```

The first line scores the corpus with each registered detector and writes one
result document per detector. The second turns those documents into a table.

## Why it is not seven commands

Most of the time a scoring run spends is not spent on the detector. Before a
single image is judged, the harness reads every record, hashes every image
against the digest its own record declares, checks that no cover and its stego
twin landed on opposite sides of the train and test split, and checks that a
stego image differs from its cover in nothing but the payload.

None of that depends on which detector you asked. It is a property of the bytes
on disk.

```
    seven separate commands          one command, seven detectors

    ┌─ establish the corpus ─┐       ┌─ establish the corpus ─┐   once
    └─ score with detector 1 ┘       ├─ score with detector 1 ┤
    ┌─ establish the corpus ─┐       ├─ score with detector 2 ┤
    └─ score with detector 2 ┘       ├─ score with detector 3 ┤
             ... x7                  │        ...             │
                                     └─ score with detector 7 ┘
```

On the Core tier that is every image behind 344,357 pairs hashed once instead
of seven times. On
a Nano tier it hardly matters. On a Core tier it is the difference between an
afternoon and a day and a half.

## What the exit code means

A zero means every detector you asked for produced a number. It never means
"most of them".

| Code | What happened |
|---|---|
| 0 | Every detector asked for produced a result |
| 3 | At least one was skipped because it is not available here, and none failed while running |
| 4 | At least one failed while running |

Codes 3 and 4 come back even when the other detectors succeeded and their
documents were written. Those documents are real measurements and they are kept;
the code is there so a script cannot read a partial baseline as a whole one.

The summary says the same thing in words:

```
2 of 3 detector(s) measured.
1 produced NO number (1 skipped, 0 failed). A report over these documents
covers only the 2 that ran.
```

## One detector failing does not lose the others

Each detector's document is written the moment that detector finishes, not at
the end of the command. A run interrupted during the fifth detector keeps the
four already measured, and running the same command again picks up where it
stopped, per detector.

Every detector gets its own records file, named after it:

```
pentimento-nano.results/zsteg.records.jsonl
pentimento-nano.results/stegexpose.records.jsonl
```

That matters more than it looks. A records file is what a resumed run reads to
know what is already done, so two detectors sharing one would mean the second
resumed from the first one's answers and reported them as its own.

## Where things go

| You typed | Documents go to | Records go to |
|---|---|---|
| One detector, no `--out` | `<corpus>.results/<detector>.json` | the same directory |
| One detector, `--out r.json` | `r.json` | the same |
| Several detectors, no `--out` | `<corpus>.results/` | the same directory |
| Several detectors, `--out DIR` | `DIR/<detector>.json` | `DIR/<detector>.records.jsonl` |

With several detectors, `--out` names a directory rather than a file. Several
result documents cannot share one file without becoming a table, and building a
table is `stegobench report`'s job rather than a second document shape.

## A directory is not a run

If a detector was measured last week and is skipped today, last week's document
is still sitting in that directory, and `stegobench report` will put its number
in the table beside today's. Nothing is deleted, because a measurement is not
this command's to throw away, but the run says so:

```
WARNING: 1 document(s) here are from an earlier run, not this one:
./pentimento-nano.results/zsteg.json. `stegobench report
./pentimento-nano.results` will include them beside today's numbers. Move them
aside if this run is meant to be the whole table.
```

## Before you start

`stegobench doctor` says what every registered tool still needs from you, in one
shape whichever kind of tool it is: an image to pull, a program to install, or an
address to export for a detector that runs as a service. `stegobench plan score
--corpus ./pentimento-core --detector all` says what the run would cost before
you commit to it.

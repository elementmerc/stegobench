# What it is

Stegobench measures how well steganalysis tools work, and ships the corpus to
measure them on.

Steganalysis is the business of looking at a picture and deciding whether
somebody hid a message inside it. Plenty of tools claim to do it; few publish a
number a stranger can reproduce.

## The problems it exists for

| Problem | What Stegobench does |
|---|---|
| Pairing discipline is described in papers rather than enforced in code, so detectors learn resaves and quantisation tables instead of payloads | Enforces it in the builder, and ships the controls that prove it |
| Detectors that print a verdict cannot be compared with detectors that print a score | Calls the function that computed the score and keeps the number |
| The standard research corpora cannot be redistributed, so a reader cannot obtain the data a result was measured on | Builds corpora from permissively licensed photographs, with each file's licence attached |

## What is in the box

**The tool you install is `stegobench`, and for scoring detectors it's the
only one you need.**

| Piece | What it does |
|---|---|
| `score` | Runs a registered detector over every image in a labelled corpus and writes a `result-v1` document naming the exact bytes it measured |
| `doctor` | Says whether this machine can run each registered tool, and makes each one flag a known planted signal and clear a known clean fixture |
| `plan` | What a run would cost, from the command you'd actually type |
| `report` | Renders results as a table with the conditions in every row, never ordered by score |
| `metrics` | AUC, detection at a fixed false-alarm rate, and confusion counts, on numbers from anywhere |
| `embed` | Drives a registered embedder, so there's something for a detector to be measured against |
| `fetch`, `list`, `describe` | Get a corpus, and see what's registered |

There's a **second, separate program** in this repository, `pentimento`,
written in Python. You need it only if you're building a corpus from scratch,
which most people never do:

| Piece | What it does |
|---|---|
| Cover fetcher | Builds a cover corpus from Wikimedia Commons, with provenance per file |
| Deduplicator | Perceptual-hash deduplication, with a corroboration rule for low-texture images |
| Arm builders | HUGO, WOW, S-UNIWARD, HILL, MiPOD, J-UNIWARD, UERD, steghide, outguess, plus an appended-data control |
| Rich-model classifier | A reference detector, for what the state of the art reaches |
| Packers | WebDataset shards, reproducibly, with each cover's licence carried onto every derivative |

## What it is not

It is not a tool for hiding things, although it can hide one. `stegobench
embed` drives a registered embedder over a cover you give it, and it exists so
you can build something for a detector to be measured against, and so the
round trip that proves an embedder works can be checked. If what you want is
to hide a file in a photo, the embedders it drives are ordinary tools you can
install and run yourself, and you would be going the long way round.

It does not tell you whether YOUR photo has something hidden in it. That is a
different question and `stegobench help scope` covers why, and where to go
instead.

It is not a leaderboard. There is no submission process and no ranking; the
output is a table you can rebuild.

It is not the corpus. A corpus built with it is published separately, with its
own documentation: [Pentimento](https://github.com/elementmerc/pentimento).

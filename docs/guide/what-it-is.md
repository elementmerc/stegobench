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

| Piece | What it does |
|---|---|
| Cover fetcher | Builds a cover corpus from Wikimedia Commons, with provenance per file |
| Deduplicator | Perceptual-hash deduplication, with a corroboration rule for low-texture images |
| Arm builders | HUGO, WOW, S-UNIWARD, HILL, MiPOD, J-UNIWARD, UERD, steghide, outguess, plus an appended-data control |
| Scorers | AUC, detection at a fixed false-alarm rate, and verdict rate, over identical bytes |
| Rich-model classifier | A reference detector, for what the state of the art reaches |
| Packers | WebDataset shards, reproducibly, with each cover's licence carried onto every derivative |

## What it is not

It does not hide anything for you. The arms exist so detectors have something
to be measured against.

It is not a leaderboard. There is no submission process and no ranking; the
output is a table you can rebuild.

It is not the corpus. A corpus built with it is published separately, with its
own documentation: [Pentimento](https://github.com/elementmerc/pentimento).

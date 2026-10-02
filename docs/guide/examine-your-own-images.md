# Examine your own images

You have images nobody has labelled and you want to know what the detectors
say about them. This is the command for that, and the first thing to be clear
about is what its answer is worth.

```sh
stegobench examine holiday --detector zsteg --detector stegexpose
```

Every detector you name sees every image you name. One row per image, one
column per detector.

Name a directory and it stands for the image files directly inside it, in
sorted order. It does not look inside subdirectories, so one mistyped path
can't turn into an hour of work. Name the files themselves if you want a
particular few, and naming a file and the folder holding it asks about it
once rather than twice.

## It is not a measurement

`examine` writes no `result-v1` document, and that is a decision rather than
unfinished work.

A result document is what comes out of a run against images whose answers were
fixed before the detector saw them, by somebody other than the detector, and
recorded beside each file. Your own photographs have none of that. There is
nothing for a detector to be right or wrong about, so there is no accuracy, no
AUC, and no number that belongs in a document.

So nothing this command prints is quotable. Not as a benchmark figure, not as
an accuracy, not as a detection rate, not in a paper and not in a report. It
is what some tools said about some files. The number you can defend comes from
labelling images and running `score`, and
[Limitations](/guide/limits) is the list of caveats even that number carries.

## A worked example

Three images, three detectors:

```sh
stegobench examine beach.jpg market.png portrait.jpg \
    --detector zsteg \
    --detector stegexpose \
    --detector aletheia-spa
```

The shape of what comes back:

```
image         zsteg   stegexpose  aletheia-spa
beach.jpg     clean   0.012       0.31
market.png    stego   0.480       0.97
portrait.jpg  clean   failed      unavailable
```

Nine cells, and they're saying four different kinds of thing.

| Cell | What it means |
|---|---|
| A number | The detector computed a score and this is it. Higher means more suspicious, on that detector's own scale and nobody else's |
| A negative number | Some detectors estimate how much payload an image carries, and an estimate can land below zero on an image carrying none. Read it as no evidence, the same as a small positive. It is not a stronger "clean" than zero, and the distance below zero means nothing |
| `stego` or `clean` | The detector only gives a verdict. Somebody else already chose the threshold and threw the number away. See [Scores, not verdicts](/guide/scores) |
| `unavailable` | This machine can't run that detector. `stegobench doctor` says what it needs |
| `failed` | It ran and produced nothing readable. `--raw` is how you find out why |

**Two scores from two detectors are not comparable.** zsteg's 0.48 and
aletheia-spa's 0.97 are numbers on two unrelated scales, and the one closer to
1 is not the more confident. Reading down a column is meaningful; reading
across a row is not. Deciding what a given detector's score means at a given
threshold is the measurement job, and it needs labelled images.

## The flags

| Flag | What it does |
|---|---|
| `--detector <NAME>`, `-d` | Required, repeatable. Every named detector sees every named image |
| `--registry <DIR>` | Read detector entries from this directory rather than wherever the search would land. Accepted by every command |
| `--timeout <SECONDS>` | Seconds one image gets before the detector is killed. Defaults to 120 |
| `--jobs <N>`, `-j` | How many images to ask about at once. Defaults to 1, because each worker starts its own container or process |
| `--raw <FILE>` | Keep what the tools actually printed |

`--raw` is the one to reach for first when a cell surprises you. A detector run
raw over one photograph typically prints pages of candidate hits with
confident-sounding names: key blocks, archives, text fragments. Nearly all of
them are noise. The table is those pages reduced to one cell; `--raw` is the
pages.

## Exit codes

| Code | What happened |
|---|---|
| 0 | Every detector answered for every image |
| 2 | A name you gave isn't a registered detector, or is registered and isn't a detector. Nothing ran |
| 3 | An image couldn't be read, or some but not all of the detectors are available here |
| 4 | A detector ran and answered nothing |
| 8 | None of the detectors you asked for is available, so the table is empty. Told apart from 3 so a script can tell a partial answer from no answer. `stegobench doctor` says what each one needs |

2 is a mistake in the command, so it's worth fixing before retrying. 3 is a
pre-flight refusal, which a script can tell apart from a failure and knows not
to retry. 4 means the tools were asked and something went wrong inside one of
them, and `--raw` is where the evidence is.

## What a row does not establish

A detector that is right nine times in ten still calls one clean image in ten
a hit. On a folder of a thousand holiday photos that's a hundred wrong alarms,
and you can't tell which hundred. These tools are built to be run over a
corpus and thresholded, not to answer yes or no about one file.

So a `stego` cell is a reason to look harder at that file. It isn't a finding,
it won't survive being challenged, and it says nothing at all about who put
anything there or when. `stegobench help scope` is the same argument where you
meet it, and [Limitations](/guide/limits) carries what a measured number does
and doesn't establish, including why a result from this harness is an assist to
a forensic validation rather than the validation itself.

## The other words for it

`stegobench check` and `stegobench scan` both run `examine`. They used to
refuse and explain the difference, because the command didn't exist; they're
kept as aliases so the words somebody guesses still land somewhere useful.
`stegobench --help` lists whichever aliases this build carries.

# The starter corpus

Six covers, twelve stego images, about 650 KB, checked into this repository so
a fresh install has something to score.

**Do not quote a number from it.** Six covers is not a measurement. An AUC over
eighteen images moves by roughly a tenth when one image changes, so a figure
from here describes which six pictures were generated and nothing about a
detector. It is here so you can watch the machinery work in the first minute
after an install, not so you can report a result.

## Running it

```
stegobench score --corpus corpora/starter --detector <name>
```

The run is marked `custom` in the result document, which `result-v1` defines as
"comparable with itself and nothing else". That marking is not a warning
somebody remembered to add: the registry entry is `demonstration = true`, and a
demonstration entry is refused by the validator if it declares the records
digest that `score` requires before it will mark a run `named`. There is no way
to configure your way past it.

## What is in it

| | |
|---|---|
| `covers/` | Six synthetic greyscale 256x256 PNGs and their records |
| `lsb-0400/` | The same six at 0.4 bits per pixel of LSB replacement |
| `lsb-0100/` | The same six at 0.1 bits per pixel |
| `manifest.jsonl` | One row per file, the shape a real tier's manifest uses |
| `build.py` | What produced all of it, from a fixed seed |

## How it is honest about itself

**The covers are generated, not photographed.** Smooth gradients plus mild
grain, which is closer to a photograph's statistics than noise or a flat field
and is still not a photograph. A threshold calibrated here does not transfer to
real images, for the same kind of reason Pentimento's own thresholds do not
transfer to a never-compressed corpus.

**The pairing rule holds by construction.** Every stego image and its cover come
out of one array through one writer, so they differ in the payload and in
nothing else. `stegobench score` checks that for itself, by comparing the format,
dimensions, bit depth and channel count of each pair, and reports `confounded` if
it ever stops being true.

**The split is a property of the cover.** Each cover is assigned to train or
test once, from its name and a fixed salt. No stego row carries a split of its
own, so it inherits its cover's and cannot contradict it, which is the leak
`score` refuses a corpus for.

**The payload is a pure function of the image.** Same cover, same tool, same
rate, same bytes, whatever else the build did or in what order. That is the rule
`generators/payloads.py` exists to enforce for the real corpus, and it is the
reason an earlier version of the real corpus could not be reproduced from its
own recorded seed.

## Rebuilding it

```
python3 corpora/starter/build.py --check    # rebuild and compare, byte for byte
python3 corpora/starter/build.py --write    # rebuild in place
```

`--check` is the one that matters: it proves the files checked in here are the
files the script produces, so nothing in the corpus can drift away from the code
that claims to have made it.

## Licence

**CC0-1.0**, a public domain dedication. Use them, publish them, build on them,
with no attribution and no share-alike condition to carry.

That is a separate grant from the repository's own licence, which is unchanged
and still AGPL-3.0-or-later over the code. The images were the copyright
holder's to waive because they are not photographs: `build.py` produces every
one of them from the fixed seed in that file, and `build.py --check` rebuilds
the shipped bytes to prove it. The reason for waiving rather than inheriting is
the person this corpus is for. A corpus under a copyleft code licence asks a
researcher to work out whether their detector's output is a derived work, and
CC0 removes the question instead of answering it.

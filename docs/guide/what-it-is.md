# What it is

stegobench is a harness for measuring how well steganalysis tools work, plus
the corpus to measure them on.

Steganalysis is the business of looking at a picture and deciding whether
somebody hid a message inside it. Plenty of tools claim to do it. Very few
publish a number that a stranger can reproduce, and the reasons have almost
nothing to do with the science.

## The two problems this exists for

**The corpora cannot be redistributed.** The standard research corpora in this
field, BOSSbase and ALASKA2 among them, either carry no readable licence at all
or carry one that forbids exactly the thing a benchmark needs: publishing
images derived from them. So a paper measures on a corpus, and the reader who
wants to check the measurement cannot obtain the same corpus on the same terms.
[Where the covers come from](/cover-source-licensing) sets out what each of
those corpora actually permits, because the answers are not what the mirrors
claim.

**The pairing discipline is described rather than enforced.** A steganalysis
measurement compares a clean image with a stego image and asks whether a
detector can tell them apart. If the two differ in any way other than the
hidden payload, a resave, a different quantisation table, a stripped timestamp,
then the detector learns that difference instead. Every paper knows this.
Almost none of them ship the code that guarantees it.

That failure is not hypothetical here. It voided a whole round of measurements
in this project, because outguess re-encodes whatever it is given at quality 75
and the clean half had been written at 95. The detector was doing well at
telling quality 75 from quality 95. [Pairing, and what breaks
it](/guide/pairing) is the longer version.

## What is in the box

| Piece | What it does |
|---|---|
| Cover fetcher | Builds a cover corpus from Wikimedia Commons under permissive licences, with per-file provenance |
| Deduplicator | Perceptual-hash deduplication, with a corroboration rule for low-texture images |
| Arm builders | Stego arms for HUGO, WOW, S-UNIWARD, HILL, MiPOD, J-UNIWARD, UERD, steghide and outguess, plus an appended-data control |
| Scorers | AUC, detection at a fixed false-alarm rate, and verdict rate, over identical bytes |
| Rich-model classifier | A reference detector, for measuring what the state of the art actually reaches |
| Packers | WebDataset shards, reproducibly, with each cover's licence carried onto every derivative |

## What it is not

It is not a hiding tool. Nothing here embeds a message for you to send; the
arms exist so that detectors have something to be measured against.

It is not a leaderboard. There is no submission process and no ranking. The
output is a table you can rebuild.

It is not finished. [Read this before quoting a number](/guide/limits) is the
honest list of what is measured, what is assumed, and what is still open.

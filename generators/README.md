# Corpus generators

Each script builds one **arm** of a labelled corpus: a directory of clean covers
and a directory of stego images, ready to be scored by any detector.

## The pairing rule

Every generator here writes the cover and the stego image from the same source
array, through the same code path, so the two differ **only** in the embedded
payload. Where a resize is involved the cover is resized first and the payload
embedded afterwards, so resampling artefacts are identical in both arms.

This is the whole reliability of the method, not a detail. If clean and stego
differ in any other way (a resave, a different quantisation table, stripped
metadata) then a detector can score well by recognising that difference, and the
result says nothing about steganography. Two separate measurements have been
lost to this class of error, so the rule is applied without exception and each
generator's docstring restates it.

## What each one does

| Script | Arm | Variable it sweeps |
|---|---|---|
| `sizesweep.py` | LSB replacement at a fixed rate | image size, 224px to 1024px |
| `payloadsweep.py` | LSB replacement at a fixed size | payload rate, 0.5 down to 0.005 bpp |
| `structural.py` | data appended after the PNG `IEND` marker | fixed; this is a floor, not a sweep |

`structural.py` exists as a control rather than a challenge. Bytes stapled to the
end of a file are the crudest hiding there is, and any tool that reads the
container will trip over them. A subject that misses LSB embedding may
reasonably be said to be facing a hard problem; a subject that also misses this
is not doing file analysis at all.

## Reproducibility

Every generator is seeded and writes `manifest.jsonl` beside its output, one
record per pair, carrying the source filename, the parameters, the number of
samples actually changed, and a sha256 of both files. Re-running with the same
seed reproduces the corpus byte for byte.

The sample count is recorded because it is the honest check that the embedding
did what was asked: a rate that produces no changed samples is a silent failure,
and the manifest makes that visible rather than leaving it to be discovered in
the results.

## Payloads

Payload bits come from a seeded generator or from `secrets.token_bytes`, so they
are uniform. Real hidden data is encrypted and therefore looks like noise;
embedding compressible text instead would make the corpus easier than reality.

## Usage

```sh
python3 generators/payloadsweep.py \
  --covers /path/to/clean/covers \
  --out    /path/to/corpus/payload \
  --count  40 \
  --size   512
```

Each writes its arms as `<out>/<arm>/{clean,stego}/NNNNN.png`. `structural.py`
is flat: `<out>/{clean,stego}/NNNNN.png`.

## Requirements

`numpy` and `Pillow`. Nothing else.

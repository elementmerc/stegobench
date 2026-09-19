# Pairing, and what breaks it

A steganalysis measurement is a comparison. You show a detector a clean image
and a stego image, and you ask whether it can tell which is which.

That only measures hiding if the two images are identical in every other
respect. If they differ in anything else, the detector will find the easier
difference, and you will publish a number that describes your pipeline rather
than the hiding.

```
    cover.png ──► [ resave at q95 ] ──► clean half
                                                     both halves must come
    cover.png ──► [ embed ] ──► [ resave at q75 ] ──► stego half
                                                     off the SAME writer

    The detector is now excellent at telling q95 from q75.
    It has learned nothing at all about the payload.
```

## This is not hypothetical

It happened here, in round 3. outguess re-encodes whatever it is handed at
quality 75. The clean half had been written by Pillow at quality 95. The arm
looked like a strong result and was measuring the quantisation table.

The arms that had the defect are kept rather than deleted, as the demonstration.

## Matching the quality is not enough

The round 3 arms were rebuilt with outguess writing at quality 95 to match the
cover, which fixed the quality difference. It did not fix the pairing, because
outguess rewrites the whole JPEG with its own encoder and the clean half was
written by Pillow. Two libraries, two sets of rounding decisions, one
difference the detector can see.

The tell was in the numbers before any control was run. Three arms spanning a
tenfold payload range produced the same separation to three decimal places. A
payload effect grows with payload; a flat line across ten times the payload is
something present in equal measure in all three.

`generators/recompression_control.py` settles it with two arms that carry no
meaningful payload at all:

| Arm | What it is | What a result on it would mean |
|---|---|---|
| `nullog` | The cover through outguess's own writer carrying **one byte**, which is 0.03% of capacity and the smallest outguess accepts. An empty payload is refused outright | The effect is the writer, not the hiding |
| `nullpillow` | The cover decoded and re-encoded by the encoder that first wrote it, carrying nothing | The detector responds to a second compression generation in general |

Run it before trusting any number measured on an arm whose tool re-encodes.
Which of the two arms moves tells you which confound you have, and an arm that
moves at 0.03% payload as much as it moves at 50% is not measuring a payload.

## What the harness does about it

**Both halves come off the same writer.** For the JPEG adaptive arms the clean
half is read and written straight back through the same library that writes the
stego half, changing nothing. Using the original encoder's file instead would
put a different encoder on each side of the pair, which is the confound above.

**The clean halves ship.** Three of them, and they are deliberately not
collapsed into one:

| Arm | What it is | Pairs with |
|---|---|---|
| `clean-grey` | The cover converted to greyscale | The spatial arms: HUGO, WOW, S-UNIWARD, HILL, MiPOD |
| `clean-jpeg` | Written back through the DCT library, coefficients untouched | The DCT arms: J-UNIWARD, UERD |
| `clean-jpeg-tools` | The cover as a JPEG, as the end-user tools were handed it | steghide, outguess, the appended-data control |

`clean-jpeg` and `clean-jpeg-tools` hold the same coefficients for the same
photograph and differ in their bytes, because different encoders wrote them.
Collapsing them would break the pairing they exist to preserve.

**Every sample records how much actually changed.** Spatial rows carry
`samples_changed` and `change_rate`; DCT rows carry `coefficients_changed`. An
arm that reports zero changes at a nominal payload is not a hard case, it is a
bug, and you can see it without opening a single image.

## The other half of the same problem: splits

A cover and its stego versions are far more alike than any two unrelated
photographs. Split a corpus at random and a cover lands in training while its
own stego copy lands in test, so the classifier recognises the photograph and
scores beautifully.

Split by cover, never by image. The published corpus ships a `SPLITS.md` with a
deterministic rule, and the sample JSON carries `source_png` precisely so you
can group by it.

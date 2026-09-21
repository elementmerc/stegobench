# Pairing, and what breaks it

A steganalysis measurement is a comparison. You show a detector a clean image
and a stego image and ask whether it can tell which is which.

That only measures hiding if the two are identical in every other respect. If
they differ in anything else, the detector finds the easier difference.

```
    cover.jpg ──► [ written at q95 ] ──────────► clean half
                                                            both halves must
    cover.jpg ──► [ embed ] ──► [ written at q75 ] ──► stego half   come off the
                                                            SAME writer
    The detector is now excellent at telling q95 from q75,
    and knows nothing at all about the payload.
```

## Matching the quality is not enough

A tool that rewrites the whole file leaves its own encoder's signature on
everything it writes. Matching the quality setting does not close that: two
JPEG libraries at quality 95 still round differently.

Measured on outguess, against a clean half written by a different library:

| Payload | AUC, mismatched clean half | AUC, writer-matched clean half |
|---|---|---|
| 0.03% of capacity | 0.360 | n/a |
| 5% | 0.361 | 0.500 |
| 20% | 0.362 | 0.502 |
| 50% | 0.360 | 0.497 |

A payload effect grows with payload. This one is flat across a 1,600-fold
range, and vanishes entirely once both halves come off the same writer.

## What the builder does about it

**A tool that rewrites the container is paired against its own writer.** Its
clean half is the cover passed through that same tool carrying the least
payload it will accept, so both halves start from the same cover and pass
through the same encoder once.

**Tools that edit coefficients in place are paired against the cover itself**,
because that file already matches.

Each sample says which it got, in its `pairing` field.

**Every sample records how much changed**, as `samples_changed` or
`coefficients_changed`. An arm reporting zero changes at a nominal payload is a
bug you can see without opening an image.

## Checking it yourself

`generators/recompression_control.py` builds two arms that carry no meaningful
payload:

| Arm | What it is | What a result on it means |
|---|---|---|
| `nullog` | Through the tool's own writer, carrying the least it accepts | The effect is the writer, not the hiding |
| `nullpillow` | Re-encoded by the encoder that first wrote it, carrying nothing | The detector responds to re-compression in general |

Run it before trusting any number from an arm whose tool re-encodes.

## Splits are the same problem

A cover and its stego versions are near-identical, so a random split puts a
cover in training and its own stego copy in test. Group by `source_png`. See
[Limitations](/guide/limits).

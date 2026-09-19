# Limitations

Each of these changes what a result means.

## It is JPEG-decompressed

Pentimento's covers are photographs, so they were JPEG compressed before they
reached the corpus. BOSSbase's covers were captured raw and never compressed.

JPEG compression leaves structure in the pixels that spatial steganalysis
features can see, so a detector measured on one is not working on the same
problem as a detector measured on the other.

**Pentimento numbers and BOSSbase numbers do not belong in the same table.**
Not as a footnote: as two different experiments.

There is no never-compressed arm here because there is no lawful route to one.
The corpora that have raw covers cannot be redistributed.

## Split by cover, never at random

A cover and its stego versions are near-identical, so a random split trains and
tests on the same photograph. See [Loading and splitting](/guide/using-it).

## The embedding is simulated

The adaptive arms embed at the theoretically optimal rate rather than through a
real syndrome-trellis code. Every sample says so in its `coding` field.

This is how these schemes are normally benchmarked and what the reference
implementations do. It still is not what a real tool produces, and the
difference makes detection slightly harder than reality.

## outguess does not fill every cover

| | |
|---|---|
| Samples per outguess arm | 8,116, not 10,000 |
| Why | outguess refuses covers it cannot fit the payload into |
| Why it matters | The refused covers are the small and the busy ones, so an outguess arm is a different cover distribution from a full arm |

## Scheme rankings invert with the cover source

Which embedding scheme is hardest to detect changes with the cover source, so a
result measured on one corpus is a result about that corpus. This is why the
arms stay separate and labelled rather than blended.

## Attribution is not optional

54% of the covers require a credit line and their stego derivatives inherit it.
It is mechanical, because the line is already written into every record, but it
is not optional. See [Licence and attribution](/guide/licence).

# Limitations

Each of these changes what a result means.

## The corpus is JPEG-decompressed

Pentimento's covers are photographs, so they were JPEG compressed before they
reached the corpus. BOSSbase's covers were captured raw and never compressed.

JPEG compression leaves structure in the pixels that spatial steganalysis
features can see, so a detector measured on one is not working on the same
problem as a detector measured on the other.

**Pentimento numbers and BOSSbase numbers do not belong in the same table.**
Not as a footnote: as two different experiments.

## Split by cover, never at random

A cover and its stego versions are near-identical. A random split puts a cover
in training and its own stego copy in test, and the classifier learns the
photograph.

Group by `source_png`. The published corpus ships a deterministic split rule.

## The embedding is simulated

The adaptive arms embed at the theoretically optimal rate rather than through a
real syndrome-trellis code. Every sample says so in its `coding` field.

This is how these schemes are normally benchmarked and it is what the reference
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

## Attribution travels with the pixels

5,429 of the 10,000 covers require attribution and their stego derivatives
inherit it. The credit line is in every sample's record, so discharging it is
mechanical, but it is not optional.

# Read this before quoting a number

Everything here is a real limit that changes what a result means. None of it is
hedging.

## The corpus is JPEG-decompressed

**Pentimento's covers were JPEG compressed at some point before they reached
us, because they are photographs from Wikimedia Commons.** BOSSbase's covers
were captured raw and never compressed.

That difference is not cosmetic. JPEG compression leaves structure in the
pixels that spatial steganalysis features can see, and a detector measured on
JPEG-decompressed covers is working on an easier or a harder problem depending
on the feature set, not on the same problem.

**So Pentimento numbers and BOSSbase numbers cannot go in the same table.** Not
as a caveat in a footnote: as two different experiments.

There is no lawful route to a never-compressed arm here, because the corpora
that have one cannot be redistributed. See [where the covers come
from](/cover-source-licensing).

## Split by cover, never by image

A cover and its stego versions are near-identical. A random split puts a cover
in training and its own stego copy in test, and the classifier learns the
photograph.

The published corpus ships a deterministic split rule and every sample carries
`source_png` so you can group by it. A nine point accuracy swing has been
measured in the literature from the split alone.

## The coding is simulated, not real

The adaptive arms embed at the theoretically optimal rate rather than through a
real syndrome-trellis code. Every sample record says so, in a `coding` field.

Simulated embedding is the standard way these schemes are benchmarked and it is
what the reference implementations do. It is still not what a real tool would
produce, and the difference is in the direction of making detection slightly
harder than reality.

## outguess does not fill every cover

The `outguess` arms hold 8,116 samples rather than 10,000. outguess refuses
covers it cannot fit the payload into, so those covers have no stego twin at
that rate.

This matters more than the missing 19% suggests: the covers it refused are not
a random sample, they are the small and the busy ones. Comparing an outguess
arm to a full arm compares two different cover distributions.

## Cover-source mismatch is real and it inverts rankings

Which embedding scheme is hardest to detect changes with the cover source. A
detector tuned on one corpus can rank schemes in a different order on another.
This is why the arms stay separate and labelled rather than blended, and why a
single-corpus result should be read as a result about that corpus.

## What has and has not been travelled

The generators, the pairing discipline and the scoring are exercised on real
corpora, and the numbers in `results/` came out of them.

The packaging and publication path has been built and run over the full corpus
but has not yet carried a public release, so treat `pack_tier.py` and
`publish_tier.py` as the least travelled code here.

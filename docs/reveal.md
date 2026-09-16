# REVEAL: the corpus to calibrate against, and the one to beat

Kombrink, van Lierop, Stolwijk, Worring, Vrijdag and Geradts. Netherlands
Forensic Institute and University of Amsterdam. *Forensic Science International:
Digital Investigation* 55 (2025), article 302006. Accepted 25 September 2025,
online 29 October 2025.

| | |
|---|---|
| **Paper DOI** | `10.1016/j.fsidi.2025.302006` |
| **Open copy** | `https://pure.uva.nl/ws/files/273178780/1-s2.0-S2666281725001465-main.pdf` |
| **Dataset** | **`https://doi.org/10.17026/PT/DITX0A`** → DANS Physical and Technical Sciences Data Station. Verified resolving 2026-09-16 |
| **Code** | `https://github.com/NetherlandsForensicInstitute/REVEAL`, GPL-3.0, last pushed 2026-07-16 |
| **Contact** | `m.kombrink@nfi.nl` |
| **Funding** | EU Horizon 2020, grant 101021687, project UNCOVER |

Note the two licences differ: the **code is GPL-3.0**, the **dataset is CC BY-SA**.

## What it contains

100,006 base images from more than 50 cameras, with a deliberately wide spread of
attributes: over 200 distinct sizes from 256x256 to 7680x4320. Those are put
through chains of 41 preprocessing options, then through **more than 50
steganography algorithms**, producing three sets (original, preprocessed, stego)
totalling more than 300,000 images. Paired by construction, and it refuses to
embed one image with several tools, in the authors' words, "in order to avoid
bias based on specific images".

It is the first steganography dataset to use many widely available end-user
software packages rather than reference implementations, and the first to include
AI-generated images alongside camera images.

## Licence, quoted

> "This dataset may be used freely under the CC-BY-SA license. Hence the dataset
> is free to use for anyone (both commercial and academic use are allowed),
> though it does require attribution and mandates that any new works derived
> from the dataset must be shared under the same or a compatible license."

**Commercial use is allowed**, which makes REVEAL almost unique among the corpora
surveyed. The sting is share-alike: anything derived from it must also be
CC BY-SA. That is why it belongs in its own arm rather than blended into a corpus
we want to licence on our own terms, and why using it to *calibrate* is free of
that problem while using it as *source material* is not.

## Using it to calibrate Stegcore

This is the more immediate value, and it carries no licensing cost at all.
Running a detector over a corpus is use, not redistribution, so nothing about
share-alike bites.

What makes REVEAL unusually good for this:

- **Over 50 real end-user tools.** Stegcore's thresholds were calibrated against
  Cassavia, BOSSbase and an ALASKA2 sample, all of which are reference
  implementations or research pipelines. REVEAL is what people actually run.
- **Payload rates to 0.00001 bpp.** Stegcore's documented weak spot is p <= 0.1,
  and the published grids in the field bottom out around 0.05. REVEAL goes four
  orders of magnitude lower, which is where S12 (the calibration ceiling found on
  2026-09-15) can actually be characterised rather than guessed at.
- **Over 200 image sizes, up to 7680x4320.** Every Stegcore calibration figure to
  date rests on 512x512.
- **AI-generated covers.** An arm nobody else has, and a distribution any deployed
  detector now meets.

The obvious first experiment is the one that has never been run: Stegcore's
per-detector thresholds (SPA 0.377, RS 0.305, WS 0.195) and its verdict ladder
against a corpus built from tools it has never seen, reported per tool and per
payload rate.

## What it deliberately leaves open

The authors exclude academic content-adaptive schemes on purpose:

> "we exclude these schemes from our dataset because we believe this comprises a
> much smaller part of steganography in the wild"

They also cover only png, jpg, bmp and gif, hold no faces, logos or number
plates, draw from one country and one season, and freeze their tool snapshot at
July 2023. They say so themselves: "REVEAL will be highly sensitive to software
versioning."

So the gap a new corpus can honestly claim is narrow and specific: **real tools
and adaptive schemes together, at camera diversity, across both spatial and JPEG
domains, as separate arms rather than a blend.** REVEAL holds one half of that
and states plainly that it is not attempting the other.

## Honest positioning

REVEAL is the serious prior art. A reviewer will know it. Any claim this project
makes should be stated as an extension of REVEAL rather than a replacement for
it, because on the axis they chose they are ahead and will stay ahead.

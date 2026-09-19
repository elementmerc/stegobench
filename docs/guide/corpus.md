# Pentimento, the corpus

A steganalysis corpus of permissively licensed cover photographs and matched
stego pairs, where every image carries its own licence and every file carries
its own checksum.

## What is in it

| | |
|---|---|
| Covers | 10,000 photographs from Wikimedia Commons |
| Stego pairs | 344,348 |
| Arms | 35, plus 3 clean arms |
| Licence | CC BY 4.0 for the collection; each file's own terms in its record |

## The arms

| Family | Tools | Rates | Domain |
|---|---|---|---|
| Adaptive, spatial | HUGO, WOW, S-UNIWARD, HILL, MiPOD | 0.05, 0.1, 0.2, 0.4 bpp | Pixels |
| Adaptive, DCT | J-UNIWARD, UERD | 0.05, 0.1, 0.2, 0.4 bpnzac | JPEG coefficients |
| End-user tools | steghide, outguess | 5%, 20%, 50% of capacity | JPEG |
| Control | Appended data after the end marker | n/a | JPEG |

The appended-data arm is the sanity check: anything claiming to detect
steganography should catch it at close to 100%. A tool that misses it is not
reading the file.

The clean arms are the other half of every pair: `clean-grey` for the spatial
arms, `clean-jpeg` for the DCT arms, `clean-jpeg-tools` and `clean-outguess`
for the tools. They are separate because they are different encodings of the
same photograph, and pairing against the wrong one measures the encoder rather
than the hiding.

## Tiers

A tier is the first *n* covers of one fixed ordering, so a smaller tier is
always a prefix of a larger one. Train on Lite, evaluate on Core, and you are
evaluating on images you trained on unless the tiers nest.

| Tier | Covers | Size, covers only / with arms | For |
|---|---|---|---|
| Nano | 200 | 64 MB / ~1 GB | Smoke tests. Downloads in seconds |
| Lite | 1,000 | 310 MB / ~20 GB | Checking an integration against real data |
| **Core** | **10,000** | **3.1 GB / ~45 GB** | **The published corpus** |
| Full | 100,000 | 31 GB / ~1.1 TB | Training |

## The format

WebDataset tar shards. A sample's parts share a basename:

```
pentimento-core-wow-0200-00000.tar
  000000.png     the image
  000000.json    its record, licence included
```

They stream without unpacking and every major dataset loader reads them. Each
part carries an index with a sha256 per shard; verify before use.

## What is in a sample's record

| Field | What it is |
|---|---|
| `source_png` | The cover this descends from. Group by it to split |
| `cover_licence` | The cover's licence, artist, credit line and source URL |
| `sha256` | The image's digest, checked at pack time |
| `samples_changed` / `coefficients_changed` | How much actually changed |
| `rate`, `rate_unit` | The nominal payload and its units |
| `coding` | Simulated at the optimal rate, not a real syndrome-trellis code |

## Licence

The collection is CC BY 4.0, which is the strictest obligation present rather
than the loosest: complying with it satisfies every file. Where a file's own
record says something looser, that applies instead.

**5,429 of 10,000 covers require attribution**, and each carries a ready-made
credit line. Stego images are derivatives and inherit their cover's terms, so
every stego sample carries its cover's licence too. A reader who downloads one
arm and never opens the cover tier still has everything the licence asks.

## Before you train on it

Read [Limitations](/guide/limits) first. The two that change results most:
split by cover rather than at random, and do not place these numbers in the
same table as BOSSbase numbers.

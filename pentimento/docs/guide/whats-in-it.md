# What is in it

| | |
|---|---|
| Covers | 10,000 photographs from Wikimedia Commons |
| Stego pairs | 344,348 |
| Arms | 35 stego, plus 4 clean |
| Format | WebDataset tar shards |
| Licence | CC BY 4.0 for the collection; each file's own terms in its record |

## The arms

| Family | Tools | Payloads | Domain |
|---|---|---|---|
| Adaptive, spatial | HUGO, WOW, S-UNIWARD, HILL, MiPOD | 0.05, 0.1, 0.2, 0.4 bpp | Pixels |
| Adaptive, DCT | J-UNIWARD, UERD | 0.05, 0.1, 0.2, 0.4 bpnzac | JPEG coefficients |
| End-user tools | steghide, outguess | 5%, 20%, 50% of capacity | JPEG |
| Control | Appended data after the end marker | n/a | JPEG |

The appended-data arm is the sanity check. Anything claiming to detect
steganography should catch it at close to 100%; a tool that misses it is not
reading the file.

## The clean arms

Every pair needs both halves, and the clean half is not always the cover.

| Arm | What it is | Pairs with |
|---|---|---|
| `clean-grey` | The cover in greyscale | The spatial arms |
| `clean-jpeg` | Written back through the DCT library, coefficients untouched | The DCT arms |
| `clean-jpeg-tools` | The cover as a JPEG, as the tools were handed it | steghide, appended data |
| `clean-outguess` | Through outguess's own writer, carrying the least it accepts | outguess |

They are separate because they are different encodings of the same photograph.
A tool that rewrites the whole file leaves its encoder's signature on
everything it writes, so pairing it against anything else measures the encoder
rather than the hiding.

## The format

A sample's parts share a basename, which is the WebDataset convention:

```
pentimento-core-wow-0200-00000.tar
  000000.png     the image
  000000.json    its record, licence included
```

Shards stream without unpacking and every major dataset loader reads them.

## What a record holds

| Field | What it is |
|---|---|
| `source_png` | The cover this descends from. Group by it to split |
| `cover_licence` | The cover's licence, artist, credit line and source URL |
| `sha256` | The image's digest, checked when it was packed |
| `rate`, `rate_unit` | The nominal payload and its units |
| `samples_changed` or `coefficients_changed` | How much actually changed |
| `pairing` | Whether the clean half came off the same writer |
| `coding` | Simulated at the optimal rate, not a real syndrome-trellis code |

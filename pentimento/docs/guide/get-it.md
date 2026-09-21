# Get it

The Core tier is 10,000 covers and 344,348 stego pairs, about 45 GB in total.
Covers and arms are packaged separately, so you can take one without the other.

## Where it is

| Host | Good for | Link |
|---|---|---|
| Internet Archive | The canonical copy. No account, no approval, permanent | [pentimento-core-v1](https://archive.org/details/pentimento-core-v1) |
| HuggingFace | Loading straight into a training pipeline | [the-malware-files/pentimento-core-v1](https://huggingface.co/datasets/the-malware-files/pentimento-core-v1) |
| Kaggle | Notebooks | [elementmerc/pentimento-core-v1](https://www.kaggle.com/datasets/elementmerc/pentimento-core-v1) |
| Academic Torrents | Bulk transfer, seeded from the Archive copy | Search `pentimento-core-v1` |

## Take only what you need

Shards are grouped one set per arm, so evaluating against WOW at 0.2 bits per
pixel does not mean downloading MiPOD to get it.

| Part | Size | What it is |
|---|---|---|
| Covers | 3.1 GB | The 10,000 clean photographs |
| One stego arm | ~1.3 GB | 10,000 images from one tool at one payload |
| Clean arms | ~3.3 GB | The other half of every pair |
| Everything | ~45 GB | 39 arms |

## One arm, start to finish

This is the whole thing: fetch an arm and its clean half, check them, read a
sample. About 2.6 GB.

```bash
BASE=https://archive.org/download/pentimento-core-v1

# The paperwork first: it is small and it tells you what the rest is.
curl -sO $BASE/README.md -O $BASE/SHA256SUMS -O $BASE/load_pentimento.py

# One arm, and the clean half it is measured against.
curl -O $BASE/pentimento-core-wow-0200-00000.tar
curl -O $BASE/pentimento-core-clean-grey-00000.tar

# Check what arrived. A shard that fails here changed in transit.
grep -E 'wow-0200-00000|clean-grey-00000' SHA256SUMS | sha256sum -c

# Read it. Nothing to install.
python3 load_pentimento.py pentimento-core-wow-0200-00000.tar
```

```
first sample: 000000
  bytes      337,041
  licence    CC BY 4.0
  cover      00000.png
500 samples in pentimento-core-wow-0200-00000.tar
```

Then see [Loading and splitting](using-it) before you train on it, because a
random split will quietly flatter your results.

## Everything

```bash
# The Archive's own client handles resume and integrity.
pip install internetarchive
ia download pentimento-core-v1 --checksum
```

On HuggingFace:

```bash
pip install huggingface_hub
hf download the-malware-files/pentimento-core-v1 --repo-type dataset --local-dir pentimento
```

## Smaller tiers

A tier is the first *n* covers of one fixed ordering, so a smaller tier is
always a prefix of a larger one. You can develop against Nano and evaluate on
Core without the two overlapping in a way that flatters your results.

| Tier | Covers | Covers only | With every arm |
|---|---|---|---|
| Nano | 200 | 66 MB | 1.0 GB |
| Lite | 1,000 | 327 MB | 4.7 GB |
| **Core** | **10,000** | **3.1 GB** | **45 GB** |

The nesting is exact rather than approximate: Lite's cover shard is the same
file, byte for byte, as Core's first cover shard.

## Check what you downloaded

`SHA256SUMS` ships beside the shards in each part, so verifying is one command:

```bash
sha256sum -c SHA256SUMS
```

Verify before use. A shard that does not match is a shard that changed in
transit, and it will read as a smaller corpus rather than as an error.

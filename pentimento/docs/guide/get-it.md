# Get it

The Core tier is 10,000 covers and 344,348 stego pairs, about 45 GB in total.
Covers and arms are packaged separately, so you can take one without the other.

## Where it is

| Host | Good for |
|---|---|
| Internet Archive | The canonical copy. No account, no approval, permanent |
| HuggingFace | Loading straight into a training pipeline |
| Kaggle | Notebooks |
| Academic Torrents | Bulk transfer, seeded from the Archive copy |

## Take only what you need

Shards are grouped one set per arm, so evaluating against WOW at 0.2 bits per
pixel does not mean downloading MiPOD to get it.

| Part | Size | What it is |
|---|---|---|
| Covers | 3.1 GB | The 10,000 clean photographs |
| One stego arm | ~1.3 GB | 10,000 images from one tool at one payload |
| Clean arms | ~3.3 GB | The other half of every pair |
| Everything | ~45 GB | 39 arms |

## Smaller tiers

A tier is the first *n* covers of one fixed ordering, so a smaller tier is
always a prefix of a larger one. You can develop against Nano and evaluate on
Core without the two overlapping in a way that flatters your results.

| Tier | Covers | Covers only / with arms |
|---|---|---|
| Nano | 200 | 64 MB / ~1 GB |
| Lite | 1,000 | 310 MB / ~20 GB |
| **Core** | **10,000** | **3.1 GB / ~45 GB** |

## Check what you downloaded

Every part carries an index with a sha256 per shard. Verify before use; a shard
that does not match its own index is a shard that changed in transit.

# Loading and splitting

## Loading

Shards are [WebDataset](https://github.com/webdataset/webdataset) tar files.
Each sample is an image and a JSON record sharing a basename:

```
pentimento-core-wow-0200-00000.tar
  000000.png     the image
  000000.json    its record, licence included
```

`load_pentimento.py` ships with the corpus and reads a shard with nothing
installed:

```bash
python3 load_pentimento.py pentimento-core-wow-0200-00000.tar
```

For training, any WebDataset loader reads the shards unchanged:

```python
import webdataset as wds

dataset = (
    wds.WebDataset("pentimento-core-wow-0200-{00000..00019}.tar")
    .decode("pil")
    .to_tuple("png", "json")
)
```

Split by cover before this, not after. The next section is why.

## Split by cover, never at random

This is the one thing that will quietly ruin a result.

A cover and its stego versions are near-identical: same scene, same camera,
same everything except a handful of changed bits. Split at random and a cover
lands in training while its own stego copy lands in test. The classifier
recognises the photograph, scores beautifully, and has learned nothing about
hiding.

```
    random split                     split by cover
    ────────────                     ──────────────
    train: 09710.png (clean)         train: every version of 09710
           09710.png (wow 0.2)  ✗
    test:  09710.png (hugo 0.1)      test:  every version of 05047
```

Group by `source_png`. The corpus ships a deterministic split rule in
`SPLITS.md`, and a nine point accuracy swing has been measured in the
literature from the split alone.

## Compare like with like

Each arm has its own clean half, named in every record's `clean` field. Use
that one rather than the cover tier, or the pair will differ in the encoder as
well as in the payload.

## Verify before you publish

Every part ships a `SHA256SUMS` beside its shards. If a number is going into a
paper, check them first:

```bash
sha256sum -c SHA256SUMS
```

It costs a minute, and a shard that arrived truncated reads as a smaller
corpus rather than as an error.

## Crediting the photographers

5,429 of the 10,000 covers are CC BY and require attribution, and stego images
inherit their cover's terms. `ATTRIBUTION.md` carries every credit line, and
`ATTRIBUTION.csv` the same list in a form you can join against. If you used one
arm rather than the whole corpus, each record names its cover under
`source_png`, so you need only the lines for the covers you actually used.

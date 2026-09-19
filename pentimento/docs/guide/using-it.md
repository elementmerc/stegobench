# Loading and splitting

## Loading

Shards are WebDataset tar files, so any loader that reads them works
unchanged. Each sample is an image and a JSON record sharing a basename.

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

Every part carries an index with a sha256 per shard. If a number is going into
a paper, check the shards against it first: it costs a minute and it is the
difference between a result and a result you can defend.

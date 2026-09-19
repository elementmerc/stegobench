# Rebuilding it yourself

The corpus is one tier of one run. Everything that made it is open, so you can
rebuild this one or make a different one.

## What you need

[Stegobench](https://github.com/elementmerc/stegobench), the harness the corpus
was built with. It fetches the covers, builds the arms, packs the shards and
derives the metadata.

## What determinism means here

| | |
|---|---|
| Pixels | Seeded per cover and per payload, so two runs produce the same images |
| Shards | Fixed member metadata, so the same corpus packs to the same bytes on any machine on any day |

A shard whose digest moves with the clock cannot be checked against a published
one, which is why the packing is fixed rather than merely repeatable.

## What you cannot reproduce exactly

The cover set itself depends on what Wikimedia Commons held when it was
fetched. Photographs are added, and occasionally removed. The manifest records
every cover's source URL and digest, so you can re-obtain the same set as long
as those files are still there, and see plainly which ones are not.

That is the same approach ImageNet takes, and it is the reason the corpus can
be redistributed at all.

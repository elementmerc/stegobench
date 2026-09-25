# Packaging

Stegobench is installed four ways that don't involve compiling it. Three of
them need a file describing the release, and none of those files is written by
hand.

| Route | What describes the release | Where it lives |
|---|---|---|
| `cargo binstall stegobench-cli` | `[package.metadata.binstall]` | `crates/stegobench-cli/Cargo.toml` |
| `brew install elementmerc/tap/stegobench` | A Homebrew formula | `elementmerc/homebrew-tap`, generated |
| `scoop install stegobench` | A Scoop manifest | `elementmerc/scoop-bucket`, generated |
| `nix run github:elementmerc/stegobench` | The flake | `flake.nix`, builds from source |

## Why the formula and the manifest are generated

Each of them is four facts about one release: the version, an archive URL, that
archive's SHA-256 digest, and the path of the binary inside it. All four change
at every tag. A file carrying all four and maintained by hand is correct right
up to the first release somebody is in a hurry during, and the failure is
quiet: the formula still installs, it just installs the version before this
one, or it fails a checksum on a stranger's machine instead of on ours.

So `tools/release/generate_packaging.py` reads the release's own `SHA256SUMS`,
which is the file the release workflow signed, and writes both. There's one
source of truth for the digests and no hand copying anywhere.

The generator refuses rather than guesses. A digest missing from `SHA256SUMS`,
an archive belonging to a different release, a build target nothing here knows
how to package: each one stops the run and says which file and which target,
because a formula with a blank in it is something a reader assumes somebody
meant to finish.

## Running it

```sh
python3 tools/release/generate_packaging.py \
    --version 1.2.3 \
    --sums dist/SHA256SUMS \
    --out /tmp/packaging
```

It writes `homebrew/stegobench.rb` and `scoop/stegobench.json` under `--out`,
and writes nothing at all if either would have been wrong. Copy the first to
`Formula/stegobench.rb` in the tap and the second to `bucket/stegobench.json`
in the bucket.

## The example in this directory

`example/` is an emitted pair so a reader can see the shape without running a
release. It's generated from `example/SHA256SUMS` and regenerated with:

```sh
python3 tools/release/generate_packaging.py \
    --version "$(cat packaging/example/VERSION)" \
    --sums packaging/example/SHA256SUMS \
    --out packaging/example
```

**Nothing in `example/` describes a real release.** The version is
`0.0.0-example` and every digest is sixty-three zeros and a counter, so
anything copied out of it fails loudly on the first download rather than
installing something wrong. `test_generate_packaging.py` regenerates the pair
and fails if what's committed has gone stale.

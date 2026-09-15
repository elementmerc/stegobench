# stegobench

A maintained, comprehensive steganography toolkit Docker image: the successor to
[DominicBreuker/stego-toolkit](https://github.com/DominicBreuker/stego-toolkit),
which has been broken since 2020.

CI-verified builds, vendored patched copies of the abandoned classics (OutGuess,
Steghide, JPHide), multi-arch, with Stegcore as the modern Rust centrepiece.

## Status

Scaffolded 2026-09-15. Nothing is built yet.

The substrate already exists and this is not greenfield: Stegcore's
`private/tools/comparators/` holds six of seven working, version-pinned
comparator images (openstego, steghide, outguess, aletheia, zsteg, stegexpose;
stegoveritas failed to build) with a docker-compose and an isolation rationale.
That directory is this repository's seed and is promoted here rather than
rewritten.

## Where heavy work runs

Building six or seven comparator images is the heaviest work on this fleet, and
it does not run on the laptop.

```
scripts/sync-to-atlas.sh                 # put the tree on the build box
scripts/hooks/heavy-run.sh <command>     # run it there
```

Both halves are installed. That is worth saying explicitly, because on
2026-09-15 sixteen of seventeen repositories on this fleet had the second and
not the first, and every dispatched build in them was running in an empty
directory.

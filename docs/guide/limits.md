# Limitations

What the harness does not do, and where a number from it needs a caveat.

## The embedding is simulated

The adaptive arm builders embed at the theoretically optimal rate rather than
through a real syndrome-trellis code. Every sample records this in its `coding`
field.

This is how these schemes are normally benchmarked and what the reference
implementations do. It still is not what a real tool produces, and the
difference makes detection slightly harder than reality.

## A tool's own capacity report is not always usable

Some tools answer a capacity query by shelling out and parsing prose, and some
of them get it wrong. outguess has been seen to report 2^61 - 1 bytes of
capacity in a 28 KB file, and to underflow to 2^64 - 1 at one quality setting while
answering correctly at another.

The builders refuse a figure larger than the file rather than trusting it, so
the failure is loud. It does mean a small number of covers are skipped for that
tool, and the manifest says which.

## Verdict-only tools give one point, not a curve

Where a detector will only say yes or no, that is all that can be recorded. It
appears as a verdict rate and is not comparable with an AUC. See
[Scores, not verdicts](/guide/scores).

## What has and has not been travelled

| Area | State |
|---|---|
| Generators, pairing, scoring | Exercised on real corpora; the published numbers came out of them |
| Packing and publishing | Built and run over a full corpus, but not yet used for a public release. Treat as the least travelled code here |
| Platforms | The generators are tested on Linux, macOS and Windows on every push. The Rust workspace runs its tests on Linux and macOS; on Windows it is compiled, test targets included, but the tests are not run, because a dozen of them drive a shell script as a stand-in detector and a suite that skips its own subject reports a green tick for having checked nothing |

## It measures; it does not rank

There is no submission process and no leaderboard. The output is a table you
can rebuild, and the corpus it was measured on is
[published separately](https://github.com/elementmerc/pentimento) so somebody
else can rebuild it too.

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
| Packing and publishing | Has taken one corpus to three public mirrors (Internet Archive, HuggingFace and Kaggle) from a single licence manifest. That is one release, not a well travelled path: the second corpus, the withdrawal of a published one and every mirror's slower failures are all still unexercised |
| Platforms | The generators are tested on Linux, macOS and Windows on every push. The Rust workspace runs its tests on Linux and macOS; on Windows it is compiled, test targets included, but the tests are not run, because a dozen of them drive a shell script as a stand-in detector and a suite that skips its own subject reports a green tick for having checked nothing |

## It measures; it does not rank

There is no submission process and no leaderboard. The output is a table you
can rebuild, and the corpus it was measured on is
[published separately](https://github.com/elementmerc/pentimento) so somebody
else can rebuild it too.

## A corpus-wide figure is weighted towards one kind of adversary

Pentimento is weighted towards academic content-adaptive schemes, which are the
hardest adversary to detect and the one a steganalysis researcher wants
measured. If you're calibrating against what actually turns up in casework, the
arms you want are the real-tool ones (steghide, outguess, openstego) and the LSB
ones, and both are scored separately so you can read them on their own.

The reasoning, the proportions and what the evidence for that says are in
`docs/design/pentimento.md`. Read it before quoting a single pooled number for
the whole corpus.

## A result is an assist, not a validation

If you work in a forensic unit, a number from this harness is evidence of
testing already performed. It is not your laboratory's validation of the
method, and it cannot be.

The UK Forensic Science Regulator's FSR-G-218 Issue 2 section 2 puts the duty
where it falls: "the onus is for the organisation using the method (i.e. the
forensic unit) to demonstrate validation, although the developer may greatly
assist the end user by providing information on the testing that has already
been performed."

A `result-v1` document is the assist half of that sentence. It records what was
measured, on which bytes, and under which conditions, so a unit can rely on it
as part of its own validation exercise. The exercise stays the unit's: the
method, the scope it is to be used in, the acceptable error margin, and the
demonstration that the method meets that margin in the unit's own hands.

FSR-G-218 Issue 2 is general guidance on validating forensic methods. It says
nothing about steganalysis in particular, and nothing here claims it does. It
is quoted because it is the clearest published statement of who owns the
obligation.

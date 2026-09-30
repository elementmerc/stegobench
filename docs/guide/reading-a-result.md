# Reading a result you did not produce

Somebody sends you a `result-v1` document with an AUC of 0.94 in it. This page
is how you decide what that number is worth, in the order worth checking.

The document is designed so you never have to take any of this on trust. Every
field below was written by the harness from what actually happened, not by the
person who ran it.

## 1. Was it measured on the corpus it says?

```sh
stegobench verify their-result.json --corpus ./the-corpus
```

`corpus.digest` names the bytes the number came from. That command recomputes
it from a corpus on your own disk and tells you whether the two are about each
other. It exits 5 if they aren't.

A match proves the document and your copy describe the same set of records.
That on its own is not enough, because the digest is computed from what the
records say: somebody can replace a stego image with an easier one, leave its
record untouched, and every digest still agrees.

So `verify` then re-reads every image and checks it against the digest its own
record states, and only then says the bytes are the bytes. It exits 5 naming
the offending records if they differ. Pass `--shallow` for a corpus too large
to re-read; it compares the records alone and says that is what it did.

An empty digest means the corpus couldn't be named at all, because at least one
of its records states no digest for its own image.

## 2. Is it comparable to anybody else's number?

Look at `declarations.configuration`.

| Value | What it means |
|---|---|
| `named` | The corpus entry declared a digest in advance, this run matched it, and every image was checked against the digest its own record states. You can put this number in a table beside somebody else's `named` run over the same corpus |
| `custom` | A perfectly good measurement that is comparable with itself. A directory nobody has registered a digest for, or a partial run |

`custom` isn't a warning. It's most runs, and it's the honest label for one.
What it rules out is quoting the figure as a tier number, because a prefix of a
corpus is not the corpus and an unregistered directory is not one either.

## 3. Could the number be measuring something other than steganography?

Two fields, and they're the ones this whole project exists for.

`declarations.pairing` says whether a stego image and the cover it came from
differ in anything but the payload:

| Value | What it means |
|---|---|
| `single-variable` | Every pair that could be compared matched on format, size, bit depth and channels. Read it as "looked and found nothing", not as proof |
| `confounded` | At least one pair differs in something else. A detector scored here is partly measuring that something else. Some arms are confounded on purpose, to demonstrate exactly this |
| `unverified` | Nothing could be compared, so no claim is made |

`declarations.split_discipline` says whether a cover and its stego twin stayed
on the same side of a train and test split. `by-cover` is the one you want;
`not-applicable` is correct for an untrained detector. A run whose corpus
violated it doesn't exist, because the harness refuses rather than producing
the inflated number.

## 4. Who or what produced it?

`declarations.self_reported` is set by the submission path and never by the
submitter. A self-reported number from the tool's own author is not worthless,
it's just a different kind of evidence.

`declarations.trained_on` names the corpus a trained detector saw. A detector
scored on what it trained on is not being measured.

## 5. Would you get the same number?

Two fields answer two different questions, and they don't always agree.

`provenance.plugins[].pinned_by` says what the digest beside it is, and so how
far the number travels:

| Value | What it means for reproducing it |
|---|---|
| `image-digest` | The digest names the container bytes that ran. Run the same command against the same image and you're running identical code, IF you can get the image: see below |
| `executable-hash` | The digest names a file on their machine. If you both built from source you'll have different hashes for the same version, and neither of you is wrong |
| `unpinned` | Nothing in the document ties the number to particular bytes. You'd be running whatever answers to that name today, which may not be what they ran |

### `image-digest` pins the run, not your ability to repeat it

A digest names bytes exactly. It doesn't mean those bytes are anywhere you can
reach. An image built on somebody's laptop and pushed to no registry pins their
run perfectly and reproduces for nobody, and the result document looks identical
either way, because nothing in a scoring run contacts a registry and a run with
no network couldn't.

This guide used to say the digest "names container bytes you can pull", and
every image this project ships was built locally and pushed nowhere. Anybody
following that sentence got an access denied error.

So if you need to reproduce somebody's number, or hand somebody else what they'd
need to reproduce yours, check that the image is actually fetchable:

```
stegobench doctor --registry-reach
```

It asks each registry whether the pinned digest is there, and says so plainly
when it can't ask, rather than passing a check it didn't perform.

`provenance.plugins[].isolation` says what that code could reach while it ran:

| Value | What it could reach |
|---|---|
| `sandbox-no-network` | A container started with no network. It saw the images it was handed and nothing else |
| `host` | A program on their machine, with their network and their privileges. Nothing constrained it |
| `remote-service` | The images went over the network to an instance they started. Nothing here constrained the tool, and the document can't vouch that the instance was the version named |
| `unstated` | Nobody recorded it. Treat it as unknown rather than as a sandbox |

The two don't move together. A detector shipped as a service has an image
digest in `image`, and the thing that actually ran was a script on the
operator's machine with the full network posting to an instance they started.
Nothing checked that the instance came from that image, so that result reads
`unpinned` and `remote-service`. One field couldn't say both without being
wrong about one of them, which is why there are two.

`provenance.host` records the operating system and architecture. Timings
certainly differ across those, and occasionally the numbers do too, where a
library dispatches on the instruction set.

`provenance.network_reachable` says whether the tool could phone home. A
container runs with no network; a locally installed program is one the operator
installed, and the harness can't speak for it.

`provenance.seed` is the seed the detector's own plugin was run with, as the
person who ran it declared it. The harness has no seed of its own, because
nothing in scoring is random. A deterministic plugin leaves the field out,
which is an answer rather than an omission. Nothing checks the value, so read
it as a note about how to set the tool up again rather than as evidence about
the number beside it.

`provenance.plugins[].determinism` says whether two runs of that tool agree at
all. Most say `unstated`, which means nobody has checked. That's different
from `nondeterministic`, which means somebody did check and it varies, and
only the second is a reason to distrust the number. Until 2026-09-30 every
document said `nondeterministic` because the value was written by the scoring
code rather than read from anywhere, so it was never a measurement.

`content_digest` covers the whole document except the two fields recording
when the run happened. Two runs of the same detector over the same corpus
produce documents that differ as text and carry the same digest, so checking
reproduction is a string comparison rather than a diff you have to know how to
read. `stegobench verify` refuses a document whose digest doesn't match its own
contents, which means something in it changed after it was written. A document
carrying no digest at all is fine: it means nobody offered one.

## Is one number hiding two?

`metrics.per_arm` breaks the headline down, one row per arm, and it's the
field to read before quoting anything. A corpus usually holds several arms,
and the headline pools them, so it describes none of them. On the starter
corpus the pooled 0.5972 is `lsb-0100` at 0.5000, which is exactly chance,
sitting beside `lsb-0400` at 0.6944. Quoting 0.5972 would tell a reader this
detector works a bit; the truth is it doesn't work at all on one arm.

Each arm is measured against the whole clean set rather than a share of it,
since a clean image belongs to no arm. The list is empty when the corpus holds
one arm, because a breakdown of one row is the headline written twice.

If somebody quotes you a single AUC over a multi-arm corpus, ask which arm.

## The short version

A number is defensible when the digest checks out, `pairing` is
`single-variable`, `split_discipline` is `by-cover` or genuinely not
applicable, `trained_on` is absent or names something other than the corpus it
was scored on, and you've read `per_arm` rather than the headline alone.
Everything else is a reason to ask one more question, not a reason to throw
the number away.

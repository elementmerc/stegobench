# The leaderboard: submission rules, before there is a table

**Nothing described in this document is live.** No table is published, no
submission has ever been accepted, and no site exists yet. This page writes
down the rules a table would run under so they can be checked and argued
about before any number is ever ranked, rather than being invented under
pressure once a submission arrives.

## Why the rules matter more than the numbers

A handful of projects have come to own the benchmark for their whole field:
MLPerf owns a table because its submission rules are strict enough that a
number in it means something specific. SWE-bench owns a prediction format
because everyone reports through it, so comparisons are free. Neither won on
features. **A benchmark's real asset is that other people's numbers get
expressed in its terms**, and that only works if the rules a submission has
to clear are public, fixed, and applied the same way to everyone, including
us.

We ship a steganography tool and we would also run the benchmark that scores
tools like ours. That conflict is real, and the honest answer to it is
mechanism, not a promise. The three divisions below exist specifically so
nobody has to take our neutrality on faith.

## The divisions

| Division | What a submission needs | What a reader may conclude |
|---|---|---|
| **Reproducible** | The corpus is publicly available, every plugin is pinned by digest (never a tag), and the seed is recorded. **We re-ran it ourselves and got the same number.** | The number is a fact, not a claim. |
| **Reported** | The submitter ran it and sent a `result-v1` document. Their setup may otherwise be private. | The number is a claim made by a named party, worth exactly as much as that party's credibility. |
| **Reference** | Produced by us, including our own rich-model baseline and Stegcore itself. | A calibration point other numbers can be read against. **Never ranked.** |

Three rules make the divisions mean something rather than being labels
anyone could apply to themselves:

- **A submission enters Reproducible only if the re-run actually happened.**
  Looking reproducible is not the same as having been reproduced. The
  qualification is the act of re-running it, not a checklist about whether it
  could in principle be re-run.
- **`self_reported: true` is set by the submission path, never by the
  submitter.** A field a submitter could set to their own advantage is not a
  field worth having.
- **Our own entries are Reference and are never ranked.** Stegcore appears on
  a table exactly like any other subject, scored by the same harness through
  the same plugin protocol as `zsteg`, and never sits at the top of a ranking
  by virtue of the fact that we run the table it appears on.

## What a submission must carry

A `result-v1` document (see `crates/stegobench-core/src/result.rs` and
`stegobench schema result-v1`), with the fields that make a division
determination possible: `subject`, `corpus` (including its digest),
`arm`, `metrics` (including `n_error`, required rather than optional, so a
partly failed run cannot be reported as a clean one), `provenance` (the
plugin's image pinned by digest, the seed, whether the run had network
access), and `declarations` (split discipline, pairing, whether the detector
trained on the corpus it was scored against, and `self_reported`).

A submission that wants the Reproducible division additionally needs the
corpus to be one we can pull ourselves and re-score, and the plugin to be an
image we can pull and run under the same sandboxing (`--network=none`,
`--cap-drop=ALL`, digest pinned) every other registry entry runs under.

## What gets rejected, and why

- **A document that does not validate against `result-v1`.** `stegobench
  validate` is the actual gate; a submission that fails it is not a matter of
  taste.
- **A mutable image tag anywhere in `provenance.plugins`.** Rejected at the
  schema level: a tag can move under the submitter without them noticing, and
  a result naming one cannot be reproduced by anyone, including the person
  who submitted it.
- **A detector declaring `trained_on` the same corpus and split it was scored
  against**, without an explicit contamination flag. A detector scored on
  what it trained on is not being measured.
- **A claim to the Reproducible division that we could not actually re-run.**
  Demoted to Reported with a stated reason, not silently dropped, so the
  submitter knows what to fix rather than wondering why their entry
  disappeared.
- **A composite or cross-corpus score.** There is no single "stegobench
  score"; a submission that tries to report one has misunderstood the table.
  See "What the table will not do" below.

## What the table will not do

- **No composite score summarising a detector across arms.** JPEG, adaptive
  and structural arms measure different capabilities; a single number across
  them would be widely quoted and mostly meaningless.
- **No ranking across corpora.** An AUC on Pentimento and an AUC on REVEAL are
  measurements of different populations, not two scores on one scale.
- **No paid tier, no verification service, no certification.** The moment
  money changes hands for a number, the table stops being worth trusting.
- **No table before the harness is usable by a stranger without our help.**
  An empty table, or one only we could have populated, is worse than no
  table at all.

## The gate on publishing the table at all

**The table is not published until entries exist from more than one party.**
A table with a handful of rows, all from the author, reads as marketing
regardless of how the divisions are labelled, and this project would rather
have no public table than that one. The realistic path to a genuine
multi-party seed is the work already in motion: the free reference panel
(Aletheia's SPA and RS, StegExpose, zsteg) plus StegaShield as a Reported
entry once FiveInsights consents to it being scored. That is a real
multi-party table on day one, once it exists; it does not exist yet.

## Where this stands today

No corpus registry entries beyond what `plugins/registry/` already carries
for tool discovery, no submission mechanism, no site, and no result has ever
been submitted by anyone outside this project.

The hosting plan behind this page is a static site over a directory of
submitted `result-v1` files, reviewed by pull request. It's left out of the
rules above because it's implementation detail rather than something a
submitter needs to know.

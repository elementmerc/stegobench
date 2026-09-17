# DEFERRED.md — the promised-later ledger

The single consolidated list of work promised for "later" that would otherwise
hide in an old session handoff. The SessionStart gate
(scripts/hooks/deferred-check.sh) surfaces the open items every session.

Format: `- [ ]` open, `- [x]` done. Group by area. The detailed trackers
(private/tech-debt.md, private/decisions.md, OPERATOR_ACTIONS.md) feed this file;
reference them, do not duplicate them.

- [ ] Mine the past session handoffs and transcripts for deferred items and record them here, grouped by area.

- **Before this repo is made public, re-check `PRIVATE_REMOTES` (2026-09-15).**
  `.baseline-hook-config` declares `origin` private, which is TRUE today: the
  GitHub repo was created private. The private-remote gate refused the first
  push because `DEFERRED.md` (this file) would have reached a remote it treats
  as public, and declaring the remote was the honest fix rather than a bypass.

  **The day this repo goes public that declaration becomes a lie and the gate
  stops protecting anything.** Going public is the plan, not a hypothetical:
  the whole point is replacing a dead public tool. So the flip must include
  either removing `origin` from `PRIVATE_REMOTES` and untracking this file, or
  deciding this ledger is fit to publish. Do not discover it afterwards.

- **One prolific uploader can dominate a random Commons sample (2026-09-16).**
  A 20 cover test run with the photograph filter returned 4 frames of
  `ISS0xx-E-xxxxx - View of Earth`, all shot on the same Nikon D4 aboard the
  space station. Each frame is a genuinely different picture, so deduplication
  correctly admits every one of them, and the corpus still ends up with a
  visible share of one camera pointed at one subject from one altitude.

  NASA has uploaded tens of thousands of these. Commons has several such bulk
  contributors, and uniform random sampling over files gives each file equal
  weight rather than each photographer or each camera.

  The fix is a cap during acquisition rather than a filter afterwards: limit how
  many covers any single uploader, camera body or title prefix may contribute,
  and record the cap in the manifest so the sampling is reproducible. It needs a
  decision on what the unit of diversity is (uploader, camera serial, or subject)
  before it can be implemented, and the v1 corpus is large enough that the effect
  may be small. Measure the concentration on the first full fetch, then decide.

## The feature cache cannot survive a concurrency change (2026-09-17)

`rich_model_baseline.py` names its cached feature files after the shard index,
`train_clean__s0.fea` through `__s7.fea`, and the shard index also decides which
images land in which file. So the cache is keyed to the shard count twice over.
Re-running the same arm with a different `--shards` recomputes everything.

Why that matters more than it sounds: it means a bad concurrency choice cannot
be cheaply reversed. On 2026-09-17 an 8 shard run turned into 128 Octave workers
on a 16 core box, consumed all swap, and took a peer's service down with it. By
the time it was measured, an hour of extraction was already cached at 8 shards,
and dropping to 3 would have thrown that hour away. The wrong setting had to be
run to completion because the cache made the right setting expensive.

The fix is to key the cache on the image rather than on the shard: one feature
file per image, or one file with an index by name, so concurrency becomes a
runtime decision that costs nothing to change. Then a concurrency mistake is a
slow hour rather than a committed one.

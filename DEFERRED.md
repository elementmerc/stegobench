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

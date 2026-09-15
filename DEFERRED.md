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

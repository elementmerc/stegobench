#!/usr/bin/env bash
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
# Stop the tier build if it starts hurting the machine, when nobody is watching.
#
# WHY THIS EXISTS
# ---------------
# A twenty hour job is running on a shared box overnight and the peer's watch has
# expired. Their advice was to size the aborts as though nobody is looking, and
# they earned the right to give it: their own watch reported CLEAN for three
# hours while its OOM check silently returned zero every poll, because
# journalctl without privilege produces nothing and an empty result counts as
# good news.
#
# So this checks that it can see what it claims to check, before it claims
# anything. A watchdog that cannot detect the thing it watches for is worse than
# no watchdog, because it converts an unmonitored risk into a monitored one that
# nobody re-examines.
#
# WHAT IT WATCHES, AND WHY THESE THREE
# ------------------------------------
#   available memory  the resource that actually ran out on 2026-09-17
#   OOM kills         the consequence, in case memory falls faster than a poll
#   brain restarts    the victim that matters to someone other than me
#
# Load average is deliberately NOT a trigger. This job is CPU bound by design
# and a high load is it working, not it misbehaving. Aborting on load would stop
# a healthy build and teach nobody anything.
set -uo pipefail

BUILD_PATTERN="${BUILD_PATTERN:-build_core_tier}"
MIN_AVAILABLE_MB="${MIN_AVAILABLE_MB:-2048}"
INTERVAL="${INTERVAL:-60}"
LOG="${LOG:-$HOME/pentimento/logs/watchdog.log}"
BRAIN_UNIT="${BRAIN_UNIT:-hephaestus-local-brain}"

say() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$LOG"; }

# ── finding the build, without finding ourselves ─────────────────────────────
# `pgrep -f "$BUILD_PATTERN"` cannot be used here and the reason is the same
# fault this file was written about. A full command line match searches every
# process's argv, and the shell wrapping this script carries the pattern in its
# own, so the query matches itself. Measured on this fleet: `pgrep -f` for a
# process that had NEVER EXISTED returned two pids and exit 0.
#
# That breaks this watchdog in both directions at once. `while pgrep -f ...`
# can never report absence, so the loop never ends; and `pkill -f ...` in the
# abort path matches the caller's own shell, so aborting the build kills the
# thing that was supposed to report the abort. Both have happened.
#
# `pgrep -x` matches the executable NAME only, so the pattern cannot appear in
# it. We then read each candidate's own /proc entry and skip our own pid. The
# result is a list of pids, and everything afterwards works on pids rather than
# on patterns.
build_pids() {
  local pid
  for pid in $(pgrep -x python3 2>/dev/null) $(pgrep -x python 2>/dev/null); do
    [ "$pid" = "$$" ] && continue
    [ -r "/proc/$pid/cmdline" ] || continue
    if tr '\0' ' ' < "/proc/$pid/cmdline" | grep -Eq -- "$1"; then
      printf '%s\n' "$pid"
    fi
  done
}

# ── self test, before watching anything ──────────────────────────────────────
# A known-bad window from 2026-09-17 contains hundreds of OOM kills. If this
# check cannot see them, it cannot see tonight's either, and saying so now is
# the whole point.
control=$(journalctl -k --since "2026-09-17 12:00" --until "2026-09-17 14:00" 2>/dev/null | grep -c "Killed process")
if [ "${control:-0}" -lt 1 ]; then
  say "REFUSING TO START: the OOM check sees 0 kills in a window known to contain"
  say "  hundreds, so it cannot detect them tonight either. Run with privilege to"
  say "  read the kernel log, or accept that this resource is unwatched and say so."
  exit 2
fi
say "self test passed: OOM check sees $control kills in the known-bad control window"

# Everything downstream watches this expression rather than the bare pattern,
# so the thing we poll for and the thing we would kill are the same set.
WATCH_RE="${WATCH_RE:-$BUILD_PATTERN|build_adaptive_arms|build_jpeg_arms}"

# A WATCHDOG THAT FINDS NOTHING MUST SAY SO, NOT EXIT QUIETLY.
# The loop below runs while a build is found, so starting before the build does
# means the body never executes and the script prints "build finished on its
# own". A watchdog started too early would report a completed build it never
# saw, which is the same fault as the OOM check that counted zero because it
# could not read the log.
if [ -z "$(build_pids "$WATCH_RE")" ]; then
  say "REFUSING TO START: no build matching '$WATCH_RE' is running. Exiting now"
  say "  would print 'build finished on its own', which is a report about a"
  say "  build this watchdog never saw. Start the build first."
  exit 2
fi

baseline=$(journalctl -k --since "10 minutes ago" 2>/dev/null | grep -c "Killed process")
say "self test passed: watching $(build_pids "$WATCH_RE" | wc -l) build process(es)"
say "watching '$WATCH_RE'; abort below ${MIN_AVAILABLE_MB}MB available, on any"
say "  new OOM kill, or if $BRAIN_UNIT restarts. Baseline kills: $baseline"

abort() {
  say "ABORTING BUILD: $1"
  local pids
  pids=$(build_pids "$WATCH_RE")
  if [ -n "$pids" ]; then
    # shellcheck disable=SC2086
    kill -TERM $pids
    sleep 3
    pids=$(build_pids "$WATCH_RE")
    # shellcheck disable=SC2086
    [ -n "$pids" ] && kill -KILL $pids
  fi
  say "build stopped. Completed arms are recorded in build-state.json and a"
  say "  re-run resumes from there rather than starting over."
  exit 1
}

while [ -n "$(build_pids "$WATCH_RE")" ]; do
  available=$(awk '/MemAvailable/ {print int($2/1024)}' /proc/meminfo)
  kills=$(journalctl -k --since "10 minutes ago" 2>/dev/null | grep -c "Killed process")
  restarts=$(systemctl show "$BRAIN_UNIT" -p NRestarts --value 2>/dev/null || echo 0)

  if [ "${available:-0}" -lt "$MIN_AVAILABLE_MB" ]; then
    abort "available memory ${available}MB is below ${MIN_AVAILABLE_MB}MB"
  fi
  if [ "${kills:-0}" -gt "${baseline:-0}" ]; then
    abort "$kills OOM kill(s) in the last 10 minutes, baseline was $baseline"
  fi
  if [ "${restarts:-0}" -gt 0 ]; then
    abort "$BRAIN_UNIT has restarted $restarts time(s)"
  fi
  sleep "$INTERVAL"
done

say "build finished on its own; watchdog exiting without having intervened"

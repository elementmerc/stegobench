#!/usr/bin/env bash
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
# Build every arm of the round 2 corpus: each cover source, crossed with each
# embedding method, swept across payload rate.
#
# The point of the cross is that a single source or a single method gives a
# number with no error bars on the thing that actually varies in the field.
# Cover source matters because a detector calibrated on curated research
# corpora meets a different distribution in a real queue; Stegcore's own
# thresholds leaked about 22% false positives on ALASKA2 in June for exactly
# that reason. Method matters because LSB replacement is the easy case every
# classical detector was built for, and measuring only replacement reports an
# upper bound on an adversary rather than an estimate of one.
#
# Usage:
#   generators/build-arms.sh <output-root> [pairs-per-arm]
#
# Layout produced:
#   <root>/<source>/<method>/<rate>/{clean,stego}/NNNNN.png
#   <root>/<source>/<method>/manifest.jsonl
#
# Idempotent: an arm whose manifest already exists is skipped, so an
# interrupted overnight run resumes rather than restarts.
set -uo pipefail

ROOT=${1:?usage: build-arms.sh <output-root> [pairs-per-arm]}
PAIRS=${2:-40}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
GEN="$HERE"

# source name -> directory of clean covers. Add a row to add a source.
#
# Set these for your own machine, or export STEGOBENCH_SOURCES as a
# space-separated list of name=path pairs. There are deliberately no defaults:
# a path that happens to exist on somebody else's disk is worse than one that
# does not, because the run succeeds against covers nobody chose.
#
#   STEGOBENCH_SOURCES="bossbase=/data/bossbase alaska2=/data/alaska2" \
#     build-arms.sh /tmp/arms 40
declare -A SOURCES=()
if [ -n "${STEGOBENCH_SOURCES:-}" ]; then
  for pair in $STEGOBENCH_SOURCES; do
    SOURCES["${pair%%=*}"]="${pair#*=}"
  done
fi
if [ ${#SOURCES[@]} -eq 0 ]; then
  echo "no cover sources configured." >&2
  echo "  Set STEGOBENCH_SOURCES=\"name=/path/to/covers ...\" or edit this file." >&2
  exit 2
fi

METHODS=(replace match)

built=0
skipped=0
missing=()

for src in "${!SOURCES[@]}"; do
  dir="${SOURCES[$src]}"
  if [ ! -d "$dir" ]; then
    missing+=("$src ($dir)")
    continue
  fi
  n=$(find "$dir" -maxdepth 1 -name '*.png' | head -n "$PAIRS" | wc -l)
  if [ "$n" -lt "$PAIRS" ]; then
    echo "note: $src has $n covers, fewer than the $PAIRS requested; using what is there" >&2
  fi
  for method in "${METHODS[@]}"; do
    out="$ROOT/$src/$method"
    if [ -f "$out/manifest.jsonl" ]; then
      echo "skip  $src/$method (already built)"
      skipped=$((skipped + 1))
      continue
    fi
    echo "build $src/$method from $dir"
    if python3 "$GEN/payloadsweep.py" \
        --covers "$dir" --out "$out" --count "$PAIRS" \
        --size 512 --method "$method" --placement spread; then
      built=$((built + 1))
    else
      echo "FAILED $src/$method" >&2
    fi
  done
done

# The structural arm is method-independent: it does not touch pixels at all, so
# it is built once per source rather than once per method.
for src in "${!SOURCES[@]}"; do
  dir="${SOURCES[$src]}"
  [ -d "$dir" ] || continue
  out="$ROOT/$src/structural"
  if [ -f "$out/manifest.jsonl" ]; then
    skipped=$((skipped + 1))
    continue
  fi
  echo "build $src/structural"
  python3 "$GEN/structural.py" --covers "$dir" --out "$out" --count "$PAIRS" \
    && built=$((built + 1))
done

echo
echo "built $built arm(s), skipped $skipped already present"
if [ ${#missing[@]} -gt 0 ]; then
  echo "sources not found, and therefore not in this corpus:"
  printf '  %s\n' "${missing[@]}"
  echo "A missing source is a gap in the result, not a warning to scroll past:"
  echo "any generalisation claim only covers the sources that actually ran."
fi

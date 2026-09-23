#!/usr/bin/env bash
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# Kaggle, which publish-all.sh cannot do.
#
# WHY THIS IS A SEPARATE SCRIPT RATHER THAN A FIFTH STEP
#
# Kaggle builds a dataset version from a WHOLE DIRECTORY through its own
# client, which does its own chunking and cannot be made to draw from the
# shared bandwidth budget the other destinations share. Running it inside
# publish-all.sh would let it take the line while the Archive and HuggingFace
# politely limited themselves; measured on 2026-09-23, a Kaggle upload running
# beside the Archive step took per-shard times from 78 seconds to 212.
#
# It is a SCRIPT rather than a paragraph in a handover because the alternative
# was memory. The unpacked-shards caveat was written into dataset-metadata.json
# on 2026-09-23 and the live description still did not carry it, because
# publishing it depended on somebody remembering to push a version by hand.
# That is the same mechanism that left "5,429 covers" on a public Archive page
# for four days.
#
# TWO THINGS THAT WILL BITE
#
#   1. Kaggle versions a whole directory, so the release directory must hold
#      the tier folders and nothing else. Anything stray goes public.
#   2. Kaggle EXTRACTS archives on upload and gives no way to refuse, so the
#      tar shards arrive as folders. That is expected, it is described in the
#      dataset description, and `load_pentimento.py --verify` is what checks
#      that copy. Do not try to defeat it.
#
# Usage:
#   tools/release/publish-kaggle.sh            # dry run, the default
#   tools/release/publish-kaggle.sh --live     # actually send
set -euo pipefail

RELEASE="${RELEASE:-$HOME/pentimento/release}"
PART="${PART:-core}"
PACKED="$RELEASE/$PART"
CREDENTIALS="${CREDENTIALS:-$HOME/catastrophic/pentimento.env}"
KAGGLE="${KAGGLE:-$HOME/stegobench-venv/bin/kaggle}"
KAGGLE_USER="${KAGGLE_USER:-elementmerc}"
LOGS="${LOGS:-$HOME/pentimento/logs}"

[ -d "$PACKED" ] || { echo "error: no packed release at $PACKED" >&2; exit 1; }
[ -x "$KAGGLE" ] || { echo "error: no kaggle client at $KAGGLE" >&2; exit 1; }

META="$PACKED/dataset-metadata.json"
[ -f "$META" ] || {
  echo "error: no dataset-metadata.json in $PACKED. It is written by" >&2
  echo "       publish_tier.py prepare and carries the title, the licence" >&2
  echo "       and the description. Without it Kaggle has nothing to name" >&2
  echo "       the dataset after." >&2
  exit 1
}

SLUG="$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['id'])" "$META")"
mkdir -p "$LOGS"

echo "packed:   $PACKED"
echo "slug:     $SLUG"
echo "message:  ${MESSAGE:=$(date -u +%Y-%m-%d) metadata and docs refresh}"

# THE DESCRIPTION IS THE PART THAT GOES STALE. Files change rarely; the prose
# around them changes every time a figure is corrected, and it is what a reader
# sees before downloading anything. Show the diff rather than assuming.
echo
echo "=== description that would be published ==="
python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['description'])" "$META"
echo "==========================================="
echo

if [ "${1:-}" != "--live" ]; then
  echo "DRY RUN. Nothing will be sent. Pass --live to publish."
  echo
  echo "It would run, from $PACKED:"
  echo "  kaggle datasets version -m '$MESSAGE' -p . -t -r skip"
  echo
  echo "  -t  KEEPS TABULAR FILES AS THEY ARE. Without it Kaggle rewrites"
  echo "      ATTRIBUTION.csv into its own CSV dialect, and that file is"
  echo "      covered by SHA256SUMS-covers, so every reader's checksum fails"
  echo "      on a file nothing is wrong with."
  exit 0
fi

echo "LIVE. This publishes a new public version of $SLUG."
(
  set -a
  # shellcheck disable=SC1090
  . "$CREDENTIALS"
  set +a
  # KAGGLE_USERNAME is not in the credentials file; only KAGGLE_KEY is.
  export KAGGLE_USERNAME="${KAGGLE_USERNAME:-$KAGGLE_USER}"
  cd "$PACKED"
  "$KAGGLE" datasets version -m "$MESSAGE" -p . -t -r skip
) 2>&1 | tee -a "$LOGS/publish-kaggle.log"

echo
echo "Now check the description actually changed, because the upload"
echo "succeeding says nothing about the prose:"
echo "  kaggle datasets metadata -p /tmp $SLUG && cat /tmp/dataset-metadata.json"

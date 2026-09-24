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
#   1. Kaggle versions a whole directory and has no exclude flag, so whatever
#      sits in the directory it is pointed at goes public. It is therefore
#      pointed at a STAGING directory holding exactly the published set, built
#      by kaggle_stage.py from the pack index and the published-file list
#      rather than from whatever happens to be on disk. Pointing it at the
#      release directory itself published ia-metadata.json, which is an
#      instruction file for the Internet Archive and no part of the corpus.
#   2. Kaggle EXTRACTS anything named `.tar` on upload and gives no way to
#      refuse. This file used to say that was expected and not to fight it.
#      Then it was measured: ten shards became 20,014 loose files, Kaggle's own
#      file listing returned HTTP 500 partway through enumerating them, and the
#      Data Card stopped rendering, so the page told visitors the corpus was
#      inaccessible while every byte of it was fine.
#
#      It extracts `.tar` and nothing else, so the staging renames the shards
#      to `.tar.bin` and they arrive whole. `kaggle_stage.py` carries the
#      measurement. Nothing a reader runs has to care, because `tarfile` and
#      `webdataset` both read a file by its content rather than its name.
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
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

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

# THE NOTEBOOK IS BUILT BEFORE THE DRY-RUN EXIT, ON PURPOSE. A dry run that
# skipped it would report success and leave the generator's first real
# exercise for the live run, which is when the line is already busy.
#
# It goes to a temporary directory rather than into $PACKED, because Kaggle
# versions that whole directory and anything stray in it goes public.
KERNEL="$(mktemp -d)"
STAGE=""
# ONE CLEANUP FOR BOTH TEMPORARY DIRECTORIES, on the way out however we leave.
# bash clears an inherited EXIT trap inside a `( )` subshell, so the subshells
# below that source the credentials do not fire this on their own exit; it runs
# once, when this shell does. INT and TERM are named too because a staged tier
# is up to 3.3 GB of hard links and a Ctrl+C should not leave it behind.
cleanup() {
  [ -n "$KERNEL" ] && rm -rf "$KERNEL"
  [ -n "$STAGE" ] && rm -rf "$STAGE"
  return 0
}
trap cleanup EXIT INT TERM

# THE PUBLISHED SET, CHOSEN BEFORE ANYTHING IS SENT. kaggle_stage.py refuses,
# loudly and with nothing staged, if a file the published set names is not in
# the release directory, and it prints what it left behind as well as what it
# took, because a silent exclusion is how the opposite fault starts.
#
# The staging directory sits BESIDE the release rather than in /tmp: hard links
# need one filesystem, and a Core tier is 3.3 GB that would otherwise be copied
# on every publish.
echo "=== what goes to Kaggle ==="
if [ "${1:-}" != "--live" ]; then
  python3 "$HERE/kaggle_stage.py" --packed "$PACKED" --dry-run
else
  STAGE="$(mktemp -d "$RELEASE/.kaggle-stage.XXXXXX")"
  python3 "$HERE/kaggle_stage.py" --packed "$PACKED" --stage "$STAGE"
fi
echo

echo "=== starter notebook ==="
python3 "$HERE/kaggle_notebook.py" --packed "$PACKED" --out "$KERNEL"
echo

if [ "${1:-}" != "--live" ]; then
  echo "DRY RUN. Nothing will be sent. Pass --live to publish."
  echo
  echo "It would run, from a staging directory holding exactly the set"
  echo "listed above:"
  echo "  kaggle datasets version -m '$MESSAGE' -p . -t -r skip"
  echo
  echo "  -t  KEEPS TABULAR FILES AS THEY ARE. Without it Kaggle rewrites"
  echo "      ATTRIBUTION.csv into its own CSV dialect, and that file is"
  echo "      covered by SHA256SUMS-covers, so every reader's checksum fails"
  echo "      on a file nothing is wrong with."
  echo
  echo "  and it would push the starter notebook built above. That was built"
  echo "  into a temporary directory which this run deletes on the way out,"
  echo "  so there is nothing to cd into: building it here is the check that"
  echo "  the generator works, and --live builds it again and pushes it."
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
  # THE STAGING DIRECTORY, NOT $PACKED. The client versions everything it
  # finds here, and here holds only what publishes plus dataset-metadata.json,
  # which it reads for the id, title, licence and description and then skips
  # rather than uploading.
  cd "$STAGE"
  "$KAGGLE" datasets version -m "$MESSAGE" -p . -t -r skip
) 2>&1 | tee -a "$LOGS/publish-kaggle.log"

echo
echo "Now check the description actually changed, because the upload"
echo "succeeding says nothing about the prose:"
echo "  kaggle datasets metadata -p /tmp $SLUG && cat /tmp/dataset-metadata.json"

# The notebook goes out in the same run as the data, for the same reason this
# file exists at all: it quotes the corpus back at the reader, so a new data
# version leaves it quoting the old one, and pushing it by hand is the
# mechanism that left a corrected description on disk while the public page
# carried the old copy.
echo
echo "=== pushing the starter notebook ==="
(
  set -a
  # shellcheck disable=SC1090
  . "$CREDENTIALS"
  set +a
  export KAGGLE_USERNAME="${KAGGLE_USERNAME:-$KAGGLE_USER}"
  cd "$KERNEL"
  "$KAGGLE" kernels push
) 2>&1 | tee -a "$LOGS/publish-kaggle.log"

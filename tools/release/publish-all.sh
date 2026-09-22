#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# Send the release to every destination, in the order the torrent requires,
# through one shared bandwidth budget.
#
# ORDER IS NOT A PREFERENCE
#
# The torrent carries the Internet Archive item as its web seed, so the Archive
# has to hold the files before the torrent means anything. Covers before arms,
# because the covers are the part somebody can use on their own.
#
# ONE BUDGET, WHATEVER THE ORDER
#
# Every step below names the same `--budget` file, so the 2 MB/s is the whole
# line rather than each destination's share of it. That holds even if these are
# later run at the same time instead of one after another.
#
# RESUMING
#
# Each destination keeps its own record of what has landed, keyed by digest, so
# re-running this skips what is already up. An interrupted run costs minutes,
# not hours, and running it twice is safe.
#
# Usage:
#   tools/release/publish-all.sh            # dry run, the default
#   tools/release/publish-all.sh --live     # actually send
set -euo pipefail

RELEASE="${RELEASE:-$HOME/pentimento/release}"
# NO -v1 on HuggingFace. An Internet Archive item is close to immutable, so its
# identifier carries the version and IA_ITEM below is right to. HuggingFace
# versions natively through git revisions, so a v1.1 corpus under a repo named
# -v1 needs either a wrong name or a second repo, which splits stars, downloads
# and every inbound link. Naming it bare also stops `pentimento-core` being
# foreclosed under this org the moment the -v1 repo is created. Tag the release
# v1.0.0 in the repo instead.
HF_REPO="${HF_REPO:-the-malware-files/pentimento-core}"
IA_ITEM="${IA_ITEM:-pentimento-core-v1}"
STATE="${STATE:-$RELEASE/.upload}"
BUDGET="$STATE/budget"
LOGS="${LOGS:-$HOME/pentimento/logs}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PYTHON="${PYTHON:-$HOME/stegobench-venv/bin/python}"

LIVE=""
[ "${1:-}" = "--live" ] && LIVE="--live"

mkdir -p "$STATE" "$LOGS"

step() {
  local packed="$1" destination="$2" item="$3" credentials="$4"
  local tag
  tag="$(basename "$packed")-$destination"
  echo
  echo "=== $tag ==="
  # Sourced in a subshell at the point of use, so no token is ever an argument
  # and none of it outlives the step that needs it.
  (
    set -a
    # shellcheck disable=SC1090
    . "$credentials"
    set +a
    "$PYTHON" "$HERE/upload_tier.py" \
      --packed "$packed" \
      --destination "$destination" \
      --item "$item" \
      --state "$STATE/$tag.json" \
      --budget "$BUDGET" \
      ${LIVE:+--live}
  ) 2>&1 | tee -a "$LOGS/publish-$tag.log"
}

if [ -z "$LIVE" ]; then
  echo "DRY RUN. Nothing will be sent. Pass --live to publish."
else
  echo "LIVE. This sends the release to public archives."
fi
echo "budget: $BUDGET   state: $STATE"

step "$RELEASE/core"      internetarchive "$IA_ITEM"  "$HOME/catastrophic/pentimento.env"
step "$RELEASE/core-arms" internetarchive "$IA_ITEM"  "$HOME/catastrophic/pentimento.env"
step "$RELEASE/core"      huggingface     "$HF_REPO"  "$HOME/catastrophic/hf-token-pentimento.env"
step "$RELEASE/core-arms" huggingface     "$HF_REPO"  "$HOME/catastrophic/hf-token-pentimento.env"

# The torrent step checks that the Archive holds every file before saying the
# seed exists, and Academic Torrents has no upload API, so it ends with an
# instruction rather than a transfer.
step "$RELEASE/core-arms" torrent "$IA_ITEM" "$HOME/catastrophic/pentimento.env"

echo
echo "Kaggle is not in this list. It builds a dataset version from a whole"
echo "directory through its own client, which does its own chunking and cannot"
echo "draw from the shared budget, so it runs on its own once these finish."

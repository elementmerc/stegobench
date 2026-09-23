#!/usr/bin/env bash
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# Run the uploader in a container that can do nothing except read the release
# and talk to one archive.
#
# WHY A CONTAINER FOR SOMETHING WE WROTE
#
# Not because the uploader is untrusted. Because the credentials are. This runs
# for six hours unattended with a write token for four public archives in its
# environment, and the smallest blast radius available is a process that holds
# no capabilities, cannot write to its own filesystem, and can see one
# directory read only.
#
# WHAT IT IS ALLOWED
#
#   --cap-drop=ALL          no capabilities at all, including NET_ADMIN, which
#                           is why the rate limit is in the reader and not in tc
#   --read-only             the root filesystem is immutable
#   --security-opt no-new-privileges
#   --network bridge        outbound only; it has to reach the archive
#   release dir  read only  except the one state file, which lives in a small
#                           writable mount of its own
#
# CREDENTIALS
#
# Sourced inside the container's shell from a file bind-mounted read only, and
# passed to the process through the environment. Never an argument: a past
# session put a token in a curl argument and it was recorded into thirteen
# permission rules in an editor's settings file, where it sat in plain text.
#
# DETACHED
#
# `-d` plus a log driver, so the run survives the laptop sleeping and the
# terminal closing. Follow it with `docker logs -f`.
#
# Usage:
#   tools/release/upload-in-container.sh <packed-dir> <destination> <item> [extra args]
#
# Nothing is sent unless `--live` is among the extra arguments. That is the
# uploader's own default and this script does not override it.
set -euo pipefail

PACKED="${1:?the packed release directory}"
DESTINATION="${2:?internetarchive, huggingface, kaggle or torrent}"
ITEM="${3:?the archive identifier or dataset slug}"
shift 3

CRED_FILE="${CRED_FILE:-$HOME/catastrophic/pentimento.env}"
IMAGE="${IMAGE:-python:3.14-slim}"
NAME="pentimento-upload-${DESTINATION}-$(date +%Y%m%d-%H%M%S)"

PACKED="$(realpath "$PACKED")"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

for path in "$PACKED" "$CRED_FILE" "$REPO/tools/release/upload_tier.py"; do
  [ -e "$path" ] || { echo "missing: $path" >&2; exit 1; }
done

# The state file is the only thing the run writes. It lives beside the release
# but is mounted separately, so the release itself can stay read only.
STATE_DIR="$(dirname "$PACKED")/.upload-state-$(basename "$PACKED")"
mkdir -p "$STATE_DIR"

echo "container:   $NAME"
echo "image:       $IMAGE"
echo "release:     $PACKED  (read only)"
echo "state:       $STATE_DIR"
echo "destination: $DESTINATION -> $ITEM"
case " $* " in
  *" --live "*) echo "mode:        LIVE. This sends bytes to a public archive." ;;
  *)            echo "mode:        DRY RUN. Nothing will be sent." ;;
esac
echo

docker run -d \
  --name "$NAME" \
  --cap-drop=ALL \
  --security-opt no-new-privileges \
  --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  --memory 512m \
  --pids-limit 64 \
  -v "$PACKED:/release:ro" \
  -v "$STATE_DIR:/state:rw" \
  -v "$REPO/tools/release/upload_tier.py:/upload_tier.py:ro" \
  -v "$CRED_FILE:/credentials.env:ro" \
  -w /release \
  "$IMAGE" \
  bash -c '
    set -euo pipefail
    # Sourced here, inside the container, so no value is ever an argument to
    # anything and none of it reaches this machine`s shell history.
    set -a; . /credentials.env; set +a
    exec python3 /upload_tier.py --packed /release --state /state/.upload-state.json '"$(printf '%q ' "--destination" "$DESTINATION" "--item" "$ITEM" "$@")"'
  '

echo "follow it with:"
echo "  docker logs -f $NAME"

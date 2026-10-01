#!/usr/bin/env bash
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# Build the toolkit image with the freshness banner filled in.
#
#   tools/toolkit/build.sh [tag]
#
# The image prints a banner naming its build date and commit so somebody can
# tell whether what they pulled is stale. Those come from --build-arg, and a
# plain `docker build .` leaves both reading "unknown", which is worse than no
# banner: it looks like an answer. The Dockerfile asked for them in a comment
# and the first build anybody ran ignored it, so this script is the mechanism
# that comment was pretending to be.
set -euo pipefail

TAG="${1:-stegobench/toolkit:local}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v docker >/dev/null 2>&1; then
    echo "build: docker is not on PATH, so there is nothing to build with" >&2
    exit 3
fi

# A build whose commit is "dirty" is one nobody can rebuild, so it is named
# that way rather than quietly stamped with the last commit's hash.
if git -C "$HERE" rev-parse --git-dir >/dev/null 2>&1; then
    VCS_REF="$(git -C "$HERE" rev-parse --short HEAD)"
    if ! git -C "$HERE" diff --quiet HEAD -- "$HERE"; then
        VCS_REF="${VCS_REF}-dirty"
    fi
else
    # Not a clone: say so rather than invent a hash. The banner's whole job is
    # letting a reader check, and a made-up ref defeats that.
    VCS_REF="not-a-git-checkout"
fi

BUILD_DATE="$(date -u +%Y-%m-%d)"

echo "building $TAG  ·  $BUILD_DATE  ·  $VCS_REF"
docker build \
    --build-arg "STEGOBENCH_BUILD_DATE=${BUILD_DATE}" \
    --build-arg "STEGOBENCH_VCS_REF=${VCS_REF}" \
    --build-arg "STEGOBENCH_IMAGE=${TAG}" \
    -t "$TAG" \
    "$HERE"

# Proving the banner is filled in is the point of the script, so it is checked
# rather than assumed: a typo in an ARG name fails silently otherwise, which is
# the exact failure this script exists to prevent.
BANNER="$(docker run --rm --network=none "$TAG" 2>&1 | head -1)"
echo "$BANNER"
if printf '%s' "$BANNER" | grep -q 'unknown'; then
    echo "build: the banner still reads 'unknown', so the build arguments did" >&2
    echo "       not reach the image. Check the ARG names in the Dockerfile." >&2
    exit 1
fi

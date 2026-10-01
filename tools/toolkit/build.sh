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
# A build run outside a clone cannot name a commit, and such an image is fit to
# use locally and NOT fit to publish, because the one field a reviewer needs is
# the one it has to leave blank. That is now a refusal rather than a note:
# pass STEGOBENCH_ALLOW_UNKNOWN_REF=1 to build one deliberately.
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

# git is no longer optional: hstego's source is fetched here rather than inside
# the build. Checked up front with everything else, because finding out four
# minutes into a twenty minute build is the failure pre-flight exists to prevent.
if ! command -v git >/dev/null 2>&1; then
    echo "build: git is not on PATH. This build fetches one dependency's source" >&2
    echo "       on this machine rather than inside the container, so git is" >&2
    echo "       required even when you are not building from a clone." >&2
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
    #
    # REFUSING RATHER THAN STAMPING IT. A platform engineer reviewing the
    # 2026-10-01 image spent a third of their time unable to answer "where did
    # this come from", because `org.opencontainers.image.revision` read
    # `not-a-git-checkout`: honest, and for their purposes identical to blank.
    # The build box here holds an rsync'd copy rather than a clone, so this
    # branch is the normal path on it and nothing was going to notice.
    #
    # The ref can still be supplied from a machine that does have the clone,
    # which is the fix rather than the workaround.
    if [ -n "${STEGOBENCH_VCS_REF:-}" ]; then
        VCS_REF="${STEGOBENCH_VCS_REF}"
    elif [ "${STEGOBENCH_ALLOW_UNKNOWN_REF:-}" = "1" ]; then
        VCS_REF="not-a-git-checkout"
        echo "build: no clone here, so this image cannot name its commit." >&2
        echo "       Building anyway because STEGOBENCH_ALLOW_UNKNOWN_REF=1." >&2
        echo "       DO NOT PUBLISH the result: a reviewer cannot trace it." >&2
    else
        echo "build: this directory is not a git clone, so the image would carry" >&2
        echo "       no commit and a reviewer could not tell where it came from." >&2
        echo >&2
        echo "  Either build from a clone, or pass the commit from one:" >&2
        echo "      STEGOBENCH_VCS_REF=\"\$(git rev-parse --short HEAD)\" $0 $TAG" >&2
        echo >&2
        echo "  To build an untraceable image on purpose, for local use only:" >&2
        echo "      STEGOBENCH_ALLOW_UNKNOWN_REF=1 $0 $TAG" >&2
        exit 3
    fi
fi

BUILD_DATE="$(date -u +%Y-%m-%d)"

# hstego's source is fetched HERE, on the host, rather than inside the build.
#
# The build used to clone it, which meant a build container reaching GitHub. On
# this build box that times out after five minutes while apt and PyPI succeed,
# so the documented build did not work and the image that existed had been made
# by hand with `--network=host`. Fetching here removes the build-time fetch
# instead of granting the build the host's network.
#
# THE PIN IS READ OUT OF THE DOCKERFILE rather than repeated here. Two copies of
# a commit hash in two files is two things to update and one of them will be
# forgotten, and the one that is forgotten decides what gets compiled.
HSTEGO_REF="$(sed -n 's/^ARG HSTEGO_REF=\([0-9a-f]\{40\}\)$/\1/p' "$HERE/Dockerfile" | head -1)"
if [ -z "$HSTEGO_REF" ]; then
    echo "build: could not read a 40 character HSTEGO_REF out of the Dockerfile." >&2
    echo "       Expected a line reading exactly: ARG HSTEGO_REF=<40 hex chars>" >&2
    echo "       A branch name there is not acceptable: this source gets compiled." >&2
    exit 1
fi

SRC="$HERE/.hstego-src"
if [ "$(git -C "$SRC" rev-parse HEAD 2>/dev/null)" = "$HSTEGO_REF" ]; then
    echo "hstego source already at $HSTEGO_REF"
else
    echo "fetching hstego at $HSTEGO_REF"
    # A shallow fetch of the one commit, so the build context stays small. Built
    # fresh rather than updated in place: a half-fetched tree from an
    # interrupted run is the one state that would compile something nobody named.
    rm -rf "$SRC.partial"
    mkdir -p "$SRC.partial"
    (
        cd "$SRC.partial"
        git init -q .
        git remote add origin https://github.com/daniellerch/hstego
        git fetch -q --depth 1 origin "$HSTEGO_REF"
        git checkout -q FETCH_HEAD
    ) || {
        echo "build: could not fetch hstego at $HSTEGO_REF." >&2
        echo "       This machine needs to reach github.com. Check the network," >&2
        echo "       and check the commit still exists upstream." >&2
        rm -rf "$SRC.partial"
        exit 3
    }
    GOT="$(git -C "$SRC.partial" rev-parse HEAD)"
    if [ "$GOT" != "$HSTEGO_REF" ]; then
        echo "build: fetched hstego is at $GOT, not the pinned $HSTEGO_REF." >&2
        rm -rf "$SRC.partial"
        exit 1
    fi
    # Moved into place only once it is complete and verified, so an interrupted
    # fetch cannot leave something the next build would quietly reuse.
    rm -rf "$SRC"
    mv "$SRC.partial" "$SRC"
fi

# A BUILD NETWORK AT THE RIGHT MTU, because the default one can be wrong and
# fail in a way that looks like the internet being down.
#
# Diagnosed on 2026-10-01 on the machine that builds these images. All of its
# traffic leaves through a tunnel with a 1280 byte MTU, nothing clamps TCP MSS
# onto that tunnel, and a container gets MTU 1500, so any first flight larger
# than 1280 bytes is black-holed. Modern curl offers a post-quantum key share,
# which pushes its TLS hello to 1565 bytes, so TLS to some hosts never completes
# and times out after exactly five minutes. Measured: at MTU 1500 a `git clone`
# fails after 150 seconds, at 1280 it succeeds in 4.
#
# Clamping MSS on the tunnel is the real fix and it needs root on the host, so
# it is not something a build script should be doing. What a build script CAN do
# is create a network at a safe MTU and build on that. 1280 is the IPv6 minimum
# and is safe over anything.
#
# This is best-effort on purpose. A machine with ordinary networking needs none
# of it, and a build that cannot create a network should still try the default
# rather than refuse: failing here would break the common case to protect the
# uncommon one.
BUILD_NET="${STEGOBENCH_BUILD_NET:-stegobench-build-mtu1280}"
NET_ARG=""
if docker network inspect "$BUILD_NET" >/dev/null 2>&1 \
   || docker network create --opt com.docker.network.driver.mtu=1280 \
        "$BUILD_NET" >/dev/null 2>&1; then
    NET_ARG="--network=$BUILD_NET"
else
    echo "build: could not use a reduced-MTU build network, continuing on the" >&2
    echo "       default. If the build hangs fetching from the network, that is" >&2
    echo "       the first thing to suspect." >&2
fi

echo "building $TAG  ·  $BUILD_DATE  ·  $VCS_REF"
# shellcheck disable=SC2086  # NET_ARG is one optional flag or empty, by design
docker build \
    $NET_ARG \
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

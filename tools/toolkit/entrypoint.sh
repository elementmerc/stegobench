#!/bin/sh
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# Dispatches to one of the bundled tools by name, and on an interactive or
# argument-less invocation prints the freshness banner the constellation plan
# (06-docker-image.md section 5) asks for: what was built, when, and from
# which commit, because the complaint about the predecessor this image
# replaces is that you cannot tell whether it is alive without trying it.
set -eu

TOOLS="steghide outguess openstego stegosuite zsteg hstego stegcore"

banner() {
    echo "stegobench toolkit  ·  built ${STEGOBENCH_BUILD_DATE:-unknown}  ·  ${STEGOBENCH_VCS_REF:-unknown}  ·  7 tools" >&2
    echo >&2
    echo "  hide a file:  steghide  outguess  openstego  stegosuite  hstego  stegcore" >&2
    echo "  look for one: zsteg  stegcore" >&2
    echo >&2
    # The image's own name, not a hardcoded one. A fixed "stegobench/toolkit"
    # here sent a reader to `docker run stegobench/toolkit zsteg --help`, which
    # answers "pull access denied ... may require 'docker login'" whenever the
    # image is tagged anything else, and that reads as a credentials problem
    # rather than a wrong name. Three names for one image is two too many.
    echo "  each runs as: docker run --rm -v \"\$PWD:/data\" ${STEGOBENCH_IMAGE:-<this image>} <tool> [args...]" >&2
    echo "  first time:   stegcore analyse /data/<file>     does anything look hidden" >&2
    echo "  any tool:     <tool> --help" >&2
}

if [ "$#" -eq 0 ]; then
    banner
    exit 0
fi

tool="$1"
shift

case "$tool" in
    steghide|outguess|openstego|stegosuite|zsteg|stegcore)
        exec "$tool" "$@"
        ;;
    hstego)
        exec /opt/hstego-venv/bin/hstego.py "$@"
        ;;
    --help|-h|help)
        banner
        exit 0
        ;;
    *)
        echo "unknown tool '$tool'. Known: $TOOLS" >&2
        exit 2
        ;;
esac

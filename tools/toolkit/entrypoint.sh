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
    echo "  versions      what version of each tool this image carries" >&2
}

if [ "$#" -eq 0 ]; then
    banner
    exit 0
fi

tool="$1"
shift

case "$tool" in
    stegcore)
        # THE WIZARD LEAKS A CONTAINER WHEN THERE IS NO KEYBOARD, so it is
        # refused here before it can start.
        #
        # Without a terminal it blocks forever waiting on a read that can
        # never return, printing nothing. It also ignores SIGTERM, so `timeout`
        # and `docker stop` kill the client and leave the container running,
        # and `--rm` never fires because --rm only reaps a container that
        # exits. Two people hit this independently within forty minutes and
        # left four containers running on a shared machine; neither saw any
        # output at all, so neither had any reason to look.
        #
        # Refusing costs a user who meant it one flag. Not refusing costs
        # everybody else a process that never ends.
        if [ "${1:-}" = "wizard" ] && [ ! -t 0 ]; then
            echo "wizard needs a keyboard, and this container does not have one." >&2
            echo >&2
            echo "  run it with:  docker run --rm -it ... ${STEGOBENCH_IMAGE:-<this image>} stegcore wizard" >&2
            echo >&2
            echo "  or skip it:   stegcore analyse  <file>          is anything hidden" >&2
            echo "                stegcore embed    --help          hide something" >&2
            echo "                stegcore extract  --help          get it back" >&2
            exit 2
        fi
        exec stegcore "$@"
        ;;
    steghide|outguess|openstego|stegosuite|zsteg)
        exec "$tool" "$@"
        ;;
    hstego)
        # Called with nothing, hstego opens a Tk window, fails because a
        # container has no display, prints a traceback, AND EXITS 0. A crash
        # reported as success is the one failure a script cannot defend
        # against, so the no-argument case is answered here instead.
        if [ "$#" -eq 0 ]; then
            /opt/hstego-venv/bin/hstego.py --help 2>/dev/null >&2 || true
            echo >&2
            echo "  hstego has no payload-rate flag: the rate is set by how big" >&2
            echo "  your message file is, and capacity is capped near 0.05 bits" >&2
            echo "  per pixel per channel. 'hstego capacity <image>' prints the" >&2
            echo "  ceiling in bytes for a given cover." >&2
            exit 2
        fi
        exec /opt/hstego-venv/bin/hstego.py "$@"
        ;;
    versions)
        # Nothing in the image named a version for any tool, so somebody who
        # needed one to cite had to override the entrypoint and run pip. That
        # is not a thing a user should have to invent.
        steghide --version 2>&1 | head -1
        outguess -h 2>&1 | grep -ai "^outguess" | head -1
        openstego --help 2>&1 | grep -ai "^openstego v" | head -1
        stegosuite --version 2>&1 | head -1
        # zsteg's own --version answers "version unknown", so ask the gem,
        # which is the thing that was actually installed and pinned.
        gem list zsteg 2>/dev/null | grep -a "^zsteg" | head -1
        stegcore --version 2>&1 | head -1
        /opt/hstego-venv/bin/pip show hstego 2>/dev/null \
            | awk '/^Name:|^Version:/ {printf "%s ", $2} END {print ""}'
        exit 0
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

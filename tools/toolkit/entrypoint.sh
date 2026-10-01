#!/bin/sh
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# Dispatches to one of the bundled tools by name, and on an interactive or
# argument-less invocation prints a freshness banner: what was built, when, and
# from which commit, because the complaint about the predecessor this image
# replaces is that you cannot tell whether it is alive without trying it.
set -eu

TOOLS="steghide outguess openstego stegosuite zsteg hstego stegcore"

# THE IMAGE DOES NOT DEFAULT SAFE AND SAYS SO, which is the honest resolution
# of a real objection rather than a dodge.
#
# A platform engineer vetting this image noted it starts as uid 0 with no USER
# directive, and that the --user in our documented alias "is not a control I
# can enforce". Both halves are right. A USER directive is still the wrong fix:
# the image cannot know the uid that owns the directory you bind-mount, so a
# fixed non-root default would leave the tools unable to write their output in
# the ordinary case, which trades a stated risk for a silent failure.
#
# So the default stays usable and stops being silent. One line on stderr, where
# it cannot corrupt output anybody parses, and only when it applies.
if [ "$(id -u)" = "0" ] && [ "${STEGOBENCH_TOOLKIT_QUIET_ROOT:-}" != "1" ]; then
    echo "note: running as root, so files written to /data will be owned by root." >&2
    echo "      pass --user \"\$(id -u):\$(id -g)\" to own your own output." >&2
fi

# Prints one version line, or says plainly that it could not get one. Silence
# was the bug: `versions` printed six lines for seven tools, exited 0, and the
# missing one was openstego, whose JVM needs a writable HOME and gets none
# under --read-only without a tmpfs. A tool list that quietly drops a tool is
# worse than one that refuses, because the reader counts the lines they got.
# Matching the tool's name alone is not enough, and the first attempt at this
# proved it: under --read-only openstego prints
# "com.openstego.desktop.OpenStegoException: ... Read-only file system", which
# contains its own name, so a name match reported a Java stack line as a
# version and still exited 0. A line is only a version if it also carries a
# version number and does not read as a failure.
version_of() {
    _name="$1"; shift
    _line=$("$@" 2>&1 \
        | grep -ai "$_name" \
        | grep -aiv -e exception -e traceback -e 'error' -e 'cannot' \
                    -e 'failed' -e 'no such' -e 'not found' \
        | grep -a -m1 -E '[0-9]+\.[0-9]+') || _line=""
    if [ -n "$_line" ]; then
        printf '%s\n' "$_line"
        return 0
    fi
    printf '%-12s COULD NOT DETERMINE\n' "$_name"
    return 1
}

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
    echo >&2
    # Where this came from and what governs it. A reviewer had to grep a URL
    # out of a binary to answer the first of these, and nothing anywhere named
    # the second. Both belong where somebody meets the image.
    echo "  source:       https://github.com/elementmerc/stegobench" >&2
    echo "  licences:     /usr/share/doc/<package>/copyright, one per package" >&2
    echo "  stegcore AUP: https://github.com/The-Malware-Files/Stegcore/blob/main/AUP.md" >&2
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
        missing=0
        version_of steghide    steghide --version          || missing=1
        version_of outguess    outguess -h                 || missing=1
        version_of openstego   openstego --help            || missing=1
        version_of stegosuite  stegosuite --version        || missing=1
        # zsteg's own --version answers "version unknown", so ask the gem,
        # which is the thing that was actually installed and pinned.
        version_of zsteg       gem list zsteg              || missing=1
        version_of stegcore    stegcore --version          || missing=1
        # hstego carries no --version at all, so its name and version are read
        # out of the installed package metadata and joined onto one line.
        hstego_v=$(/opt/hstego-venv/bin/pip show hstego 2>/dev/null \
            | awk '/^Name:|^Version:/ {printf "%s ", $2} END {print ""}' \
            | sed 's/ *$//') || hstego_v=""
        if [ -n "$hstego_v" ]; then
            printf '%s\n' "$hstego_v"
        else
            printf '%-12s COULD NOT DETERMINE\n' hstego
            missing=1
        fi
        if [ "$missing" -ne 0 ]; then
            echo >&2
            echo "  At least one tool could not report a version, and the usual cause is a" >&2
            echo "  read-only container with no writable temporary directory: openstego runs" >&2
            echo "  on a JVM that writes to HOME, which is /tmp here. If you passed" >&2
            echo "  --read-only, add:  --tmpfs /tmp:rw,noexec,nosuid" >&2
            exit 1
        fi
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

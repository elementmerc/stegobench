#!/usr/bin/env bash
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# Publish the plugin images the registry pins, WITHOUT rebuilding any of them.
#
# WHY THIS IS A SCRIPT AND NOT A WORKFLOW
#
# The images exist only on the machines that built them. A GitHub runner cannot
# see them, and the only way a runner could produce them is to build them
# again, which is the one operation that must never happen here: a rebuild
# produces different bytes, so it produces a different manifest digest, and
# every registry entry naming the old one becomes false. So this runs from the
# machine that holds the bytes and it transfers them. It has no build path at
# all, by construction rather than by instruction.
#
# WHY A DIGEST SURVIVES THE MOVE
#
# Measured 2026-10-02 rather than assumed. An image's manifest is content
# addressed: the digest is the sha256 of the manifest bytes, those bytes name
# the config and layer blobs by digest in turn, and a registry stores and serves
# exactly what it was given. A manifest uploaded to a registry comes back under
# the same digest and `docker pull repo@sha256:...` resolves it. What changes is
# the NAME in front of the digest, which is why publishing these needs an edit
# to each registry entry's `image.reference`: same digest, new repository.
#
# WHAT DECIDES WHETHER A GIVEN IMAGE GOES
#
# Not this script, and not its licence field. Each entry carries
# `redistribution` and `redistribution_reason`, and this reads them. The three
# outcomes are publish, skip with the entry's own recorded reason, and refuse to
# proceed because the entry does not say. An entry that says nothing is not a
# yes, and the refusal is the whole run rather than that one image, because a
# skip inside a loop is how a partial publish comes to look like a complete one.
#
# WHY EMBEDDERS ARE NOT IN THE DEFAULT SET
#
# Three of the registered embedders are copyleft and two of those are GPL-2.0,
# whose section 3 offers no equivalent of GPL-3 section 6(d): a link to
# somebody else's server is neither accompanying the binary with source nor our
# own written offer, so publishing those two needs a corresponding source
# answer that does not exist yet. Nobody needs an embedder to reproduce a
# DETECTOR number, which is what the published results are: the embedders built
# the corpus, the corpus is already published, and a reader checking a detection
# figure needs the detectors and the corpus. So detectors are the default set
# and `--kind` has to be given to widen it. Widening it does not weaken the
# licence check: each entry still has to say, and an entry whose compliance
# answer is not settled says `forbidden` or says nothing, and either way nothing
# is published.
#
# EXIT CODES
#
#   0  finished, and every image in scope was either published or skipped with
#      a reason the entry recorded
#   1  usage, or a pre-flight that failed before anything was read
#   2  an entry in scope does not say whether its image may be served
#   3  an image the registry names is not on this machine
#   4  a published digest does not match the pinned one, which means a registry
#      entry is now wrong
#
# Needs bash 4 or newer for the arrays below, docker, jq and the stegobench
# binary. Point `STEGOBENCH` at the binary if it is not on PATH, which it is
# not on a machine that only ever builds and runs the plugin images.

set -euo pipefail

STEGOBENCH="${STEGOBENCH:-stegobench}"

HOST="ghcr.io"
NAMESPACE=""
REGISTRY_DIR=""
KIND="detectors"
TAG="pinned"
PUBLISH=0

usage() {
    cat <<'TEXT'
Publish the plugin images the registry pins, without rebuilding them.

Usage:
  publish-plugin-images.sh --namespace NAME [options]

Options:
  --namespace NAME   the namespace to publish under, for example a GitHub
                     organisation or user. Required.
  --host HOST        the registry host. Default ghcr.io
  --registry DIR     the tool registry to read. Default is whatever the
                     stegobench binary resolves, which may be the copy
                     compiled into it rather than a checkout.
  --kind KIND        detectors, embedders, or all. Default detectors.
                     See the note on embedders at the top of this file.
  --tag TAG          the tag each image is pushed under. Default pinned.
                     The DIGEST is the pin; the tag is only a handle, because
                     a push needs a name to send.
  --publish          actually push. Without it this is a dry run that reads
                     everything, decides everything, and sends nothing.
  --help             this text.

The decision to publish any one image comes from that entry's own
`redistribution` field, never from this script.
TEXT
}

fail() {
    # Every refusal names the thing it is refusing and what to do, because a
    # release path that says "failed" sends somebody reading logs instead of
    # reading the entry.
    local code="$1"; shift
    printf '\n%s\n' "ABORTED: $*" >&2
    exit "${code}"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --namespace) NAMESPACE="${2:-}"; shift 2 ;;
        --host)      HOST="${2:-}"; shift 2 ;;
        --registry)  REGISTRY_DIR="${2:-}"; shift 2 ;;
        --kind)      KIND="${2:-}"; shift 2 ;;
        --tag)       TAG="${2:-}"; shift 2 ;;
        --publish)   PUBLISH=1; shift ;;
        --help|-h)   usage; exit 0 ;;
        *)           usage >&2; fail 1 "unknown argument $1" ;;
    esac
done

# ── Pre-flight, before anything is read and long before anything is sent ────

[ -n "${NAMESPACE}" ] || { usage >&2; fail 1 "--namespace is required"; }

# The namespace becomes part of a repository name, and a repository name is
# lowercase with a restricted alphabet. Checked here rather than left to the
# registry, which answers a malformed name with a 404 that reads like a
# missing package.
case "${NAMESPACE}" in
    *[^a-z0-9._-]*) fail 1 "namespace ${NAMESPACE} is not a valid repository path segment: lowercase letters, digits, dot, underscore and hyphen only" ;;
esac

case "${KIND}" in
    detectors|embedders|all) ;;
    *) fail 1 "--kind must be detectors, embedders or all, not ${KIND}" ;;
esac

for program in docker jq; do
    command -v "${program}" > /dev/null \
        || fail 1 "${program} is not on PATH, and this needs it"
done
command -v "${STEGOBENCH}" > /dev/null \
    || fail 1 "${STEGOBENCH} is not on PATH. Set STEGOBENCH to the binary's path"

docker version --format '{{.Server.Version}}' > /dev/null 2>&1 \
    || fail 1 "no container runtime answered. The images live in its store, so there is nothing to read without it"

# ── What the registry says ──────────────────────────────────────────────────
#
# One source of truth. A second list of images in this file would be a second
# answer to the question of which images exist, and the first time the two
# disagreed the wrong one would be published.

registry_args=()
if [ -n "${REGISTRY_DIR}" ]; then
    [ -d "${REGISTRY_DIR}" ] || fail 1 "--registry ${REGISTRY_DIR} is not a directory"
    registry_args+=(--registry "${REGISTRY_DIR}")
fi

listing=""
listing="$("${STEGOBENCH}" list --json "${registry_args[@]+"${registry_args[@]}"}")" \
    || fail 1 "${STEGOBENCH} list --json would not run. Fix that before publishing anything"
[ -n "${listing}" ] \
    || fail 1 "${STEGOBENCH} list --json printed nothing"

printf '%s' "${listing}" | jq -e '.tools' > /dev/null 2>&1 \
    || fail 1 "the listing has no tools array, so this is not the document this expects"

# WHICH registry was read, printed rather than assumed. A binary carries a
# registry compiled into it, so running this on a box with an old binary and no
# checkout would publish against an old set of digests and nothing would say
# so. The answer belongs in the summary a human reads.
registry_source="$(printf '%s' "${listing}" | jq -r '.registry.source // "unknown"')"
registry_path="$(printf '%s' "${listing}" | jq -r '.registry.path // "compiled into the binary"')"

case "${KIND}" in
    detectors) kind_filter='["detector"]' ;;
    embedders) kind_filter='["embedder"]' ;;
    all)       kind_filter='["detector","embedder"]' ;;
esac

# One row per unique IMAGE, not per tool: two entries share one image where one
# tool is registered twice for two measurements (aletheia-rs and aletheia-spa),
# and pushing it twice would be a second upload of bytes already there.
#
# Entries sharing an image must agree about whether it may be served. If they
# disagree, that is a contradiction in the registry rather than something to
# resolve here, so both answers are carried through and the decision below
# refuses.
rows=""
rows="$(printf '%s' "${listing}" | jq -r --argjson kinds "${kind_filter}" '
    [ .tools[]
      | select(.image != null)
      | select(.kind as $k | $kinds | index($k))
      | { name: .name,
          reference: .image.reference,
          decision: (.redistribution // "unstated"),
          # FLATTENED HERE, NOT WHERE IT IS PRINTED.
          #
          # `redistribution_reason` is a multi line basic string in the TOML, so
          # it arrives carrying real newlines, and `@tsv` has to escape those to
          # keep one record on one line: it turns each into the two characters
          # backslash and n. Printed with %s those come out literally, so the
          # stegashield skip line read as one long sentence with four `\n` in
          # the middle of it.
          #
          # The row genuinely has to stay on one line, so the prose is folded to
          # single spaces before it ever reaches `@tsv`. Unfolding it afterwards
          # at the printf would mean reading an escape sequence back out of a
          # field, which is guessing about where a line break was meant.
          reason: ((.redistribution_reason // "")
                   | gsub("[[:space:]]+"; " ")
                   | sub("^ "; "") | sub(" $"; "")) } ]
    | group_by(.reference)
    | map({
        reference: .[0].reference,
        names: (map(.name) | sort | join(", ")),
        decisions: (map(.decision) | unique),
        reasons: (map(.reason) | map(select(. != "")) | unique)
      })
    | sort_by(.reference)
    | .[]
    | [ .reference,
        .names,
        (.decisions | join(" and ")),
        (.reasons | join(" / ")) ]
    | @tsv
')" || fail 1 "could not read the images out of the listing"

if [ -z "${rows}" ]; then
    printf 'No %s in this registry name an image, so there is nothing to publish.\n' "${KIND}"
    exit 0
fi

# ── Decide, then act, one image at a time ───────────────────────────────────

published=()
skipped=()
already=()
would=()
# Images served on a recorded decision rather than on a grant, carried
# separately so the summary can say so. See the marker below.
by_decision=()

printf 'Registry: %s (%s)\n' "${registry_source}" "${registry_path}"
printf 'Target:   %s/%s/<name>:%s\n' "${HOST}" "${NAMESPACE}" "${TAG}"
printf 'Scope:    %s\n' "${KIND}"
if [ "${PUBLISH}" -eq 1 ]; then
    printf 'Mode:     PUBLISHING. Every push is checked against the pinned digest before the next one starts.\n\n'
else
    printf 'Mode:     dry run. Nothing will be sent. Pass --publish to send.\n\n'
fi

while IFS=$'\t' read -r reference names decision reason; do
    [ -n "${reference}" ] || continue

    grounds=""
    repo="${reference%@*}"
    digest="${reference#*@}"
    name="${repo##*/}"
    target="${HOST}/${NAMESPACE}/${name}"

    # The last gate before publication, and one comparison. The registry reader
    # already refuses a mutable tag; a reference that reached here without a
    # digest would be published as whatever the tag meant today.
    case "${reference}" in
        *@sha256:*) ;;
        *) fail 2 "${names} names ${reference}, which is not pinned by digest. Nothing here can publish bytes it cannot name" ;;
    esac

    # WHAT COUNTS AS A YES
    #
    # `permitted` is a grant. `mirrored-by-decision` is no grant and a recorded
    # decision to serve a copy anyway, which is a real position with a reason
    # and a condition beside it. Both mean a copy may be served, which is the
    # question a publish path asks; `Redistribution::may_be_served` in
    # stegobench-core is the same rule and is the one to change if this ever
    # moves. `forbidden` is a no with a reason. Everything else, including an
    # entry that carries no field at all and including a value this script has
    # never heard of, is "does not say", and does not get a guess.
    case "${decision}" in
        permitted|mirrored-by-decision)
            if [ -z "${reason}" ]; then
                fail 2 "${names} says ${decision} and gives no reason. A publish decision with no reason beside it is the defect this project exists to correct: fill in redistribution_reason"
            fi
            # THE TWO YESES ARE NOT THE SAME YES, AND THE OUTPUT HAS TO SAY SO.
            #
            # `permitted` is a grant somebody read. `mirrored-by-decision` is no
            # grant at all and a decision to serve a copy anyway, which is a
            # real position with a reason and a condition beside it. Printing
            # them identically would make the second invisible at exactly the
            # moment it matters, which is the moment the copy goes up. So it is
            # marked on the line and carried into its own group in the summary,
            # where the reason travels with it.
            if [ "${decision}" = "mirrored-by-decision" ]; then
                grounds="  (no grant: served on a recorded decision)"
                by_decision+=("${name}|${reason}")
            else
                grounds=""
            fi
            ;;
        forbidden)
            printf 'skip  %-16s %s\n' "${name}" "${reason:-forbidden, and the entry gives no reason}"
            skipped+=("${name}|${reason:-forbidden, with no reason recorded}")
            continue
            ;;
        *)
            fail 2 "$(printf '%s names %s and its entry does not say whether that image may be served (redistribution is %s).\n  Fill in redistribution and redistribution_reason on that entry. Corpora entries have carried both since the beginning and are the pattern.\n  An unstated answer is refused rather than assumed, and the whole run stops rather than skipping one image, because a skip in a loop is how a partial publish looks complete.' "${names}" "${reference}" "${decision}")"
            ;;
    esac

    # ── Is it already there, at the right digest? ──────────────────────────
    #
    # Asked before the local store is searched, so a re-run after an
    # interruption is a read and a sentence rather than a second upload of
    # bytes the registry already has. A failure here is NOT read as "not
    # published": it is also what an unauthenticated read of a private package
    # looks like, so the only safe conclusion is "not known to be there", and
    # the push below is a no-op server side when it turns out it was.
    if docker manifest inspect "${target}@${digest}" > /dev/null 2>&1; then
        printf 'ok    %-16s already published at the pinned digest\n' "${name}"
        already+=("${name}|${target}@${digest}")
        continue
    fi

    # ── Find the bytes. NEVER build them. ─────────────────────────────────
    #
    # Matched on repository and digest together, because the digest is what the
    # entry pins and the repository is what makes it the image the entry means.
    local_ref=""
    local_ref="$(docker images --digests --format '{{.Repository}}@{{.Digest}} {{.Repository}}:{{.Tag}}' \
        | awk -v want="${reference}" '$1 == want { print $2; exit }')" || true

    if [ -z "${local_ref}" ]; then
        fail 3 "$(printf '%s is not on this machine.\n  It is named by %s and nothing here will build it: a rebuild produces different bytes, so a different digest, and every registry entry naming the old one would become false.\n  Run this on the machine that holds it. What was searched is the output of docker images with the digests column; note that a daemon without the containerd image store records no digest for an image it built, in which case the image is here and cannot be matched this way.' "${name}" "${reference}")"
    fi

    if [ "${PUBLISH}" -eq 0 ]; then
        printf 'would %-16s %s  ->  %s:%s%s\n' "${name}" "${local_ref}" "${target}" "${TAG}" "${grounds}"
        would+=("${name}|${target}@${digest}")
        continue
    fi

    printf 'push  %-16s %s%s\n' "${name}" "${local_ref}" "${grounds}"
    docker tag "${local_ref}" "${target}:${TAG}" \
        || fail 1 "could not tag ${local_ref} as ${target}:${TAG}"
    docker push "${target}:${TAG}" \
        || fail 1 "pushing ${target}:${TAG} failed. Nothing after this image was attempted"

    # ── THE GATE. Every push, before the next one starts. ─────────────────
    #
    # Success on a push says what the client sent, not what the registry holds.
    # The whole value of these entries is that the digest in them names the
    # bytes a reader will pull, so a digest that moved is not a warning, it is
    # a registry full of false statements. The run stops here.
    #
    # Filtered by repository rather than taking the first RepoDigest: this image
    # is now tagged under its original name and under the target, so the list
    # has more than one entry and the first is whichever docker wrote first.
    published_digest=""
    published_digest="$(docker image inspect "${target}:${TAG}" \
        --format '{{range .RepoDigests}}{{println .}}{{end}}' \
        | grep -F "${target}@" \
        | head -n 1 \
        | cut -d@ -f2)" || true

    if [ -z "${published_digest}" ]; then
        fail 4 "$(printf '%s was pushed and the runtime recorded no digest for %s, so this cannot confirm what the registry holds.\n  Check the registry by hand before pushing anything else.' "${name}" "${target}")"
    fi
    if [ "${published_digest}" != "${digest}" ]; then
        fail 4 "$(printf '%s published as %s but its registry entry pins %s.\n  EVERY REGISTRY ENTRY NAMING THIS IMAGE IS NOW WRONG, and a reader following one would pull bytes that are not the bytes a number was measured on.\n  Nothing after this image was attempted. Work out what rewrote the manifest before you push again; the usual cause is something rebuilding or re-wrapping the image rather than transferring it.' "${name}" "${published_digest}" "${digest}")"
    fi

    printf 'ok    %-16s %s@%s\n' "${name}" "${target}" "${digest}"
    published+=("${name}|${target}@${digest}")
done <<< "${rows}"

# ── The summary, written to be pasted ──────────────────────────────────────

printf '\n'
# Said in both modes and worded for both. "Published against" would be a lie in
# a dry run, and the registry that was read is the fact worth carrying either
# way: an old binary carries an old set of digests and nothing else would say so.
printf 'Read the %s registry (%s).\n\n' "${registry_source}" "${registry_path}"

report_group() {
    local heading="$1"; shift
    # No rows means no heading. A heading with nothing under it reads as
    # something having gone missing.
    [ "$#" -gt 0 ] || return 0
    printf '%s\n' "${heading}"
    local row
    for row in "$@"; do
        printf '  * %s: %s\n' "${row%%|*}" "${row#*|}"
    done
    printf '\n'
}

if [ "${PUBLISH}" -eq 1 ]; then
    report_group "Published, and each digest checked against the entry that pins it:" "${published[@]+"${published[@]}"}"
    report_group "Already published at the pinned digest, so nothing was sent:" "${already[@]+"${already[@]}"}"
else
    report_group "Would publish:" "${would[@]+"${would[@]}"}"
    report_group "Already published at the pinned digest:" "${already[@]+"${already[@]}"}"
fi
report_group "Not published, for the reason the entry records:" "${skipped[@]+"${skipped[@]}"}"
# Last, and never folded into the list above it. Everything else in this summary
# is a name and a digest; this is the one group a reader has to actually read,
# so it closes the report rather than scrolling past in the middle of it.
report_group "Served on a recorded decision rather than on a grant. No licence permits these, somebody decided, and the reason and the condition are:" "${by_decision[@]+"${by_decision[@]}"}"

if [ "${PUBLISH}" -eq 1 ] && [ "${#published[@]}" -gt 0 ]; then
    cat <<'TEXT'
NEXT, AND THE RUN IS NOT FINISHED WITHOUT IT

Each entry above still names the old repository. The digest travels and the
name does not, so every entry whose image moved needs its `image.reference`
edited to the new repository with THE SAME DIGEST copied across, in one commit
with nothing else in it.

Then make the package public, because a package nobody can read is a pin
nobody can follow, and record the result in tools/release/channels.toml.
TEXT
fi

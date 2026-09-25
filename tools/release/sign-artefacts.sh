#!/usr/bin/env bash
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# Sign every file in a staged release directory, then verify what was signed.
#
# WHY THIS IS A SCRIPT AND NOT A BLOCK OF YAML
#
# A `run:` block in a workflow can only be tested by running the workflow, and
# this one runs on a tag push, which is the one moment nobody wants to be
# finding out. As a script it is executed by `test_sign_artefacts.py` against a
# stub `cosign`, offline, on every CI run, including the partial failure case
# that matters most: cosign dying on the fourth of nine files.
#
# WHY IT VERIFIES WHAT IT JUST SIGNED
#
# `cosign sign-blob` returning 0 says the call succeeded. It does not say the
# signature on disk verifies against the certificate on disk, and a release
# carrying signatures that do not verify is worse than one carrying none: it
# invites a reader to run a check, fail it, and conclude the download was
# tampered with. So every signature is verified here, in the same job, before
# anything reaches a release page.
#
# WHY SHA256SUMS IS SIGNED TOO, EXPLICITLY
#
# It is the file an attacker would most want to swap. A checksum list proves
# the bytes have not changed since somebody wrote the list; whoever can serve
# you a tarball can serve you a matching list. Signing it is what turns it from
# an integrity statement into a provenance one, so this script refuses to run
# at all if SHA256SUMS is not in the directory.
#
# USAGE
#
#   tools/release/sign-artefacts.sh dist
#
# Environment, both with defaults that name this repository:
#   COSIGN_IDENTITY_REGEXP  the workflow identity the certificate must carry
#   COSIGN_OIDC_ISSUER      the OIDC issuer the certificate must come from
set -euo pipefail

readonly DEFAULT_IDENTITY='^https://github\.com/elementmerc/stegobench/\.github/workflows/release\.yml@refs/tags/v'
readonly DEFAULT_ISSUER='https://token.actions.githubusercontent.com'

identity="${COSIGN_IDENTITY_REGEXP:-${DEFAULT_IDENTITY}}"
issuer="${COSIGN_OIDC_ISSUER:-${DEFAULT_ISSUER}}"

die() {
    # `::error::` is what turns a line red in an Actions log. Harmless
    # everywhere else, which is why it is unconditional.
    echo "::error::sign-artefacts: $*" >&2
    exit 1
}

if [ "$#" -ne 1 ]; then
    die "expected exactly one argument, the staged release directory. Usage: tools/release/sign-artefacts.sh dist"
fi

dist="$1"

# ── Pre-flight, before a single signature exists ────────────────────────────
#
# Every one of these would otherwise surface halfway through the loop, with
# some files signed and some not.

[ -d "${dist}" ] || die "no such directory: ${dist}"

command -v cosign >/dev/null 2>&1 \
    || die "cosign is not on PATH. The release workflow installs it; if you are running this by hand, install cosign first."

[ -s "${dist}/SHA256SUMS" ] \
    || die "${dist}/SHA256SUMS is missing or empty. That file is the one an attacker would most want to swap, so signing a release without it is refused."

# `-print` and basename rather than GNU find's `-printf '%P\n'`, because this
# script is exercised by the test suite on macOS too and bsdfind has no
# `-printf`. Sorting whole paths orders them by name, the prefix being the
# same for all of them.
artefacts=()
while IFS= read -r path; do
    artefacts+=("$(basename "${path}")")
done < <(find "${dist}" -maxdepth 1 -type f \
    ! -name '*.sig' ! -name '*.pem' -print | LC_ALL=C sort)

[ "${#artefacts[@]}" -gt 0 ] || die "${dist} holds no artefacts to sign."

printf 'Signing %d artefacts in %s\n' "${#artefacts[@]}" "${dist}"

# ── Sign ────────────────────────────────────────────────────────────────────

for name in "${artefacts[@]}"; do
    target="${dist}/${name}"
    printf '  sign  %s\n' "${name}"
    cosign sign-blob --yes \
        --output-signature "${target}.sig" \
        --output-certificate "${target}.pem" \
        "${target}" \
        || die "cosign could not sign ${name}. Nothing has been published; re-run the release once the cause is fixed."
    [ -s "${target}.sig" ] || die "cosign reported success for ${name} but wrote an empty signature."
    [ -s "${target}.pem" ] || die "cosign reported success for ${name} but wrote an empty certificate."
done

# ── Verify, against the identity a downloader will check ────────────────────

for name in "${artefacts[@]}"; do
    target="${dist}/${name}"
    printf '  check %s\n' "${name}"
    cosign verify-blob \
        --certificate "${target}.pem" \
        --signature "${target}.sig" \
        --certificate-identity-regexp "${identity}" \
        --certificate-oidc-issuer "${issuer}" \
        "${target}" \
        || die "the signature for ${name} does not verify against ${identity}. Nothing has been published."
done

# ── Reconcile ───────────────────────────────────────────────────────────────
#
# A count, because the loops above prove each file they visited was signed and
# nothing proves the loops visited every file. Cheap, and it catches a filename
# the find above quietly dropped.

signatures="$(find "${dist}" -maxdepth 1 -type f -name '*.sig' | wc -l | tr -d ' ')"
certificates="$(find "${dist}" -maxdepth 1 -type f -name '*.pem' | wc -l | tr -d ' ')"

if [ "${signatures}" -ne "${#artefacts[@]}" ] || [ "${certificates}" -ne "${#artefacts[@]}" ]; then
    die "counted ${signatures} signatures and ${certificates} certificates for ${#artefacts[@]} artefacts. Refusing to continue."
fi

printf 'Signed and verified %d artefacts, SHA256SUMS among them.\n' "${#artefacts[@]}"

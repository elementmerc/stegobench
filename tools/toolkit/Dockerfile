# The default toolkit image: core + tools tier (operator decision D3,
# 2026-09-21). Carries steghide, outguess, openstego, stegosuite, zsteg,
# hstego and stegcore, everything measured at 9 MB to 1.41 GB, so a user who
# wants to run steghide is not made to download a deep learning stack first.
# See ../IMAGES.md for how each of these numbers was measured and ./README.md
# for how to add Aletheia on top once you need it.
#
# STEGEXPOSE IS NOT HERE, AND IT USED TO BE.
#
# Its upstream repository grants no licence: no LICENSE file, no COPYING file
# and no licence section in its README, checked inside the pinned image on
# 2026-09-30. Archiving a public repository does not grant one either, so the
# default under copyright is that none is given and we may not redistribute
# it. Running it locally is unaffected and its registry entry stays, now
# recording `licence = "none-granted"` rather than the GPL-3.0 it used to
# claim with nothing behind it. Operator decision, 2026-10-01: dropped from
# the default image out of respect for an author who has stopped maintaining
# it, and available on demand like every other tool outside this image.
#
# STEGCORE IS HERE, AND IT WAS ALWAYS MEANT TO BE.
#
# Its registry entry has said so since it was written: "Bundled without
# question: a binary this size is free against an image measured in
# hundreds." The decision was recorded there and never reached this file, so
# the image that was supposed to carry it never mentioned it. Dropping
# StegExpose is what made the omission matter: it left this image with one
# detector, and that one has never successfully run under a sandbox that
# denies it a writable /tmp.
#
# BUILT AND PARTLY TESTED. A six-tool build of this file, everything except
# stegcore, was built on 2026-10-01 and measured 1.59 GB. openstego and
# steghide each went through an embed and extract round trip that recovered
# the payload byte for byte, and zsteg ran. outguess, stegosuite and hstego
# install and are on the PATH; nobody has put a file through them here.
# ./README.md carries the table.
#
# REDISTRIBUTING THIS IMAGE CARRIES A SOURCE OFFER. steghide, openstego and
# stegosuite are GPL and stegcore is AGPL, so pushing this image to a public
# registry obliges us to offer the corresponding source alongside it. See
# ./SOURCE-OFFER.md, which has to be published with the image rather than
# written afterwards.
#
# All base images pinned by digest, never by tag, per baseline Section 5 and
# ../IMAGES.md. Every digest here is copied from an existing per-tool
# Dockerfile in this repository rather than invented.
#
# WHY ONE DEBIAN BASE RATHER THAN FOUR
# -------------------------------------
# The seven tools this image bundles were built, one per image, on three
# different bases: Debian (steghide, outguess, stegosuite), Eclipse Temurin
# (openstego, stegexpose) and Ruby (zsteg), plus hstego's own Python image
# with a hand-built libjpeg. Stacking four unrelated base images into one
# combined image is not how a single filesystem works: a combined image needs
# ONE base, with every runtime installed into it.
#
# Debian is that base, for two reasons. First, it is already the base for
# three of the seven tools, so those three need no behavioural change at all.
# Second, building hstego's runtime (its private libjpeg 9e, its Python
# virtual environment) in a builder stage that shares this exact Debian
# digest guarantees glibc and library ABI compatibility with the final image,
# which building it against a *different* Debian release, or against Alpine's
# musl, would not.
#
# The Java runtime for openstego and StegExpose, and Ruby for zsteg, are
# installed via Debian's own `apt` rather than by reusing the Temurin or
# Ruby upstream images, for the same reason: one apt transaction on one base
# produces one set of shared libraries, which is what makes the combined
# image smaller than the sum of the seven originals (they already share some
# base layers; this shares all of them).

ARG DEBIAN_DIGEST=debian@sha256:520c7157b4b1c3c46fa56bac0e748ea21d60956c0512b300e3b16d8757f68e2d

# ---------------------------------------------------------------------------
# Stage 0: Stegcore's published image, used only as a source to copy from.
# ---------------------------------------------------------------------------
# STEGCORE, COPIED OUT OF ITS OWN PUBLISHED IMAGE RATHER THAN BUILT HERE.
#
# Three routes were examined on 2026-10-01 and only this one yields bytes a
# reader can obtain:
#
#   - Building from source here would produce a binary whose hash pins bytes
#     nobody else has, inside an image we hand to other people. That is the
#     defect the `pinned_by` wording was corrected for in `3ca0465`, one level
#     down, and it would be ours this time rather than inherited.
#   - The published .deb carries `usr/bin/stegcore-gui` and nothing else: the
#     Tauri desktop app, 15.7 MB, no command line tool. Verified by unpacking
#     the v4.1.0 asset whose sha256 matches its published checksum.
#   - This image is built by Stegcore's own release pipeline, from
#     `cargo build --release -p stegcore-cli`, and is the ONLY published
#     artefact that contains the command line binary at all.
#
# Pinned by digest, never by tag, the same rule every other base here follows.
# `latest`, `4.1` and `4.1.0` all pointed at this digest when it was recorded.
ARG STEGCORE_DIGEST=ghcr.io/the-malware-files/stegcore@sha256:14b7a78ad9c0d92d0ac1db0bab9df84d1f24ce3e48da735566898900e4601046
FROM ${STEGCORE_DIGEST} AS stegcore-source

# ---------------------------------------------------------------------------
# Stage 1: build hstego's private libjpeg 9e and its Python virtual
# environment, against the SAME Debian base and digest as the final image.
# Copied verbatim from tools/hstego/Dockerfile's reasoning: hstego's JPEG
# extension reads struct fields (block_size, min_DCT_h_scaled_size,
# DCT_h_scaled_size) that exist in the Independent JPEG Group's libjpeg from
# v8 onward and NOT in the libjpeg-turbo jpeg62 ABI every mainstream
# distribution ships, so `libjpeg-dev` fails to compile against it.
# ---------------------------------------------------------------------------
FROM ${DEBIAN_DIGEST} AS hstego-builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        gcc g++ libc6-dev make curl ca-certificates git \
        python3 python3-venv python3-dev \
    && rm -rf /var/lib/apt/lists/*

ARG LIBJPEG_VERSION=9e
RUN curl -fsSL "https://www.ijg.org/files/jpegsrc.v${LIBJPEG_VERSION}.tar.gz" \
        -o /tmp/jpegsrc.tar.gz \
    && tar -xzf /tmp/jpegsrc.tar.gz -C /tmp \
    && cd "/tmp/jpeg-${LIBJPEG_VERSION}" \
    && ./configure --prefix=/usr/local --enable-shared \
    && make -j"$(nproc)" && make install \
    # make install ships only the public headers. hstego's extension includes
    # <jpegint.h>, libjpeg's INTERNAL header, and reaching into those
    # internals is the whole reason this tool can read DCT coefficients.
    && cp -n /tmp/jpeg-${LIBJPEG_VERSION}/*.h /usr/local/include/ \
    && ldconfig \
    && rm -rf /tmp/jpeg*

ENV CFLAGS="-I/usr/local/include" LDFLAGS="-L/usr/local/lib" \
    LD_LIBRARY_PATH="/usr/local/lib"

# PINNED TO A COMMIT, NOT A BRANCH, and that distinction is load bearing here
# in a way it is not for most dependencies: pip does not install a wheel from
# this URL, it clones the repository and COMPILES TWO NATIVE C EXTENSIONS
# (hstego_jpeg_toolbox_extension, hstego_stc_extension) into the image. A
# branch name means the C code that executes on a user's machine is whatever
# was on that branch the day somebody rebuilt, under the same image tag, with
# no way for a reader to pin it from outside and no reproducible build.
#
# This commit is not a guess. It is the commit the 2026-10-01 image actually
# resolved `master` to, read back out of that image's own
# `hstego-0.6.1.dist-info/direct_url.json`, so pinning it reproduces the build
# that was tested rather than selecting a new one. Moving it is a deliberate
# act with a self-test behind it, which is the point.
#
# Raised by rung 5 of the 2026-10-01 journey round, by a platform engineer who
# found it in `direct_url.json` and said the resolved commit being recorded
# after the fact is not the same as pinning it before.
ARG HSTEGO_REF=bf71f6e0d7faaa632ad8a988c0393e94bfd13b2a

# THE SOURCE ARRIVES IN THE BUILD CONTEXT RATHER THAN BEING CLONED IN HERE, and
# that is not a workaround for a flaky network even though a flaky network is
# what forced it.
#
# `pip install git+https://...` made the build reach GitHub from inside a build
# container. On the machine that builds these images that fetch times out after
# five minutes while apt and PyPI succeed in the same layer, measured twice on
# 2026-10-01, so `build.sh` could not build this image at all and the image that
# existed had been made by hand with `--network=host`. Handing a build the
# host's network to clone and compile somebody else's source is the opposite of
# what baseline Section 5 asks for, so widening the hole was not the fix.
#
# `build.sh` now fetches exactly this commit on the host, where the network
# works, checks that the tree really is at it, and leaves it in the build
# context. The build-time fetch is gone rather than permitted, which also
# removes the second of the two unpinned network fetches rung 5 objected to: the
# only remaining one is openstego's .deb, and that is checked by hash.
#
# The commit is written into the image because pip records a `file://` URL for a
# local install, so `direct_url.json` no longer carries the commit the way it
# did when this cloned. The check that matters moved to the host, where it can
# inspect the actual tree rather than pip's note about it.
#
# Running `docker build` directly will now fail on this COPY, by design. Use
# ./build.sh, which the Dockerfile header already told you to use.
COPY .hstego-src /tmp/hstego-src
RUN python3 -m venv /opt/hstego-venv \
    && /opt/hstego-venv/bin/pip install --no-cache-dir \
        imageio scipy pycryptodome numba pillow \
    && /opt/hstego-venv/bin/pip install --no-cache-dir /tmp/hstego-src \
    && printf '%s\n' "${HSTEGO_REF}" > /opt/hstego-venv/HSTEGO_COMMIT \
    && rm -rf /tmp/hstego-src

# ---------------------------------------------------------------------------
# Stage 2: the toolkit image itself.
# ---------------------------------------------------------------------------
FROM ${DEBIAN_DIGEST}

LABEL org.opencontainers.image.title="stegobench toolkit" \
      org.opencontainers.image.description="steghide, outguess, openstego, stegosuite, zsteg, hstego, stegcore in one image" \
      # Seven tools under five licences, packaged side by side rather than
      # merged, so no single SPDX identifier describes the image. The
      # expression names every one of them; ./README.md has the per-tool
      # table and ./SOURCE-OFFER.md the obligations that follow from it.
      # A single identifier here would be a claim about the whole image that
      # is not true of any of its parts.
      org.opencontainers.image.licenses="AGPL-3.0-or-later AND GPL-2.0-only AND GPL-3.0-only AND BSD-3-Clause AND MIT" \
      # WHERE THIS CAME FROM, and it was missing until 2026-10-01.
      #
      # A platform engineer vetting this image for a network spent a third of a
      # forty-five minute review unable to answer "where did this come from",
      # and answered it in the end by running `grep` over the 9.74 MB stegcore
      # binary and finding a URL in its strings. These three labels are where
      # that question is supposed to be answered, and an image that leaves them
      # off is not being modest, it is withholding the one field a reviewer
      # needs. `revision` is set from ARG below rather than here, because it
      # changes per build.
      org.opencontainers.image.source="https://github.com/elementmerc/stegobench" \
      org.opencontainers.image.url="https://github.com/elementmerc/stegobench" \
      org.opencontainers.image.vendor="Daniel Iwugo" \
      org.opencontainers.image.documentation="https://github.com/elementmerc/stegobench/blob/dev/docs/guide/toolkit.md"

# curl is build-time only: it fetches OpenStego's .deb, which is a GitHub
# release asset rather than a Debian package. It is purged after use, matching
# the pattern the per-tool Dockerfiles already use for the same reason (see
# tools/zsteg/Dockerfile).
ARG OPENSTEGO_VERSION=0.8.6
ARG ZSTEG_VERSION=0.2.13

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        # the three tools that are plain Debian packages
        steghide outguess stegosuite \
        # A headless JRE for openstego, which is Java.
        #
        # 21 RATHER THAN THE 17 THE STANDALONE IMAGE RUNS, and that is a real
        # difference rather than a tidy-up. `tools/openstego/Dockerfile` is
        # built on Eclipse Temurin and reports `Temurin-17.0.20+8`; this image
        # is on Debian trixie, which offers openjdk 21 and 25 and no 17 at
        # all, so staying on 17 would mean a second base image for one tool
        # and the whole point of this file is one base. 21 is the current LTS
        # and the smaller step of the two.
        #
        # A JVM major version is not a cosmetic choice for a tool somebody
        # measures with, so openstego's embed and extract round trip is run
        # against this image rather than assumed to still work.
        openjdk-21-jre-headless \
        # zsteg is pure Ruby; `file` is a runtime dependency (zsteg shells
        # out to libmagic to identify the carrier, per tools/zsteg/Dockerfile)
        ruby file \
        # build-time only, purged below
        git curl ca-certificates build-essential \
        # runtime libs numba/scipy wheels commonly need beyond what a
        # manylinux wheel bundles itself
        libgomp1 \
        python3 \
        # hstego's entry point opens with `from PIL import Image, ImageTk`
        # before it parses an argument, so tkinter is a hard import on the
        # command line path and not only in whatever GUI it has. Without it
        # every invocation, `--help` included, ends in a ModuleNotFoundError.
        # The standalone image gets this from `tk` in its builder; the venv
        # copied in here brings Pillow but not the interpreter's own tkinter,
        # which is a system package.
        python3-tk \
    && gem install zsteg -v "${ZSTEG_VERSION}" --no-document \
    # Retried because this is the one fetch in the build that leaves the
    # distribution's own mirrors, and it is the one that fails: two builds in a
    # row died here on "Recv failure: Connection reset by peer" after every
    # earlier step had succeeded. An unretried single GET of a release asset
    # makes a twenty minute build depend on one TCP connection to one host.
    # --retry-all-errors because a reset is not an HTTP status and the plain
    # --retry does not cover it.
    && curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors --max-time 300 \
        -o /tmp/openstego.deb \
        "https://github.com/syvaidya/openstego/releases/download/openstego-${OPENSTEGO_VERSION}/openstego_${OPENSTEGO_VERSION}-1_all.deb" \
    # THE VERSION BEING PINNED DOES NOT PIN THE BYTES, which is the whole
    # reason this line exists. Everything else in this build arrives from
    # Debian's mirrors, signed, and apt verifies it. This one file is a GitHub
    # release asset, and a release asset can be replaced by whoever owns the
    # repository without the version string changing, so `0.8.6` names an
    # intention rather than a payload.
    #
    # The digest is not taken from the release page, which would only restate
    # whatever is being served now. It was computed from the asset and then
    # checked against the image that was already built and tested: all six
    # files the package installs match dpkg's own recorded md5sums inside that
    # image, so this hash identifies the bytes this project has actually run.
    #
    # Raised by rung 5 of the 2026-10-01 journey round, which stopped on
    # exactly this layer and said it is why they read image history at all.
    && echo "821c4a1e190e994659b05a6c8142d14568204675b431c9e7278b379934a1d34a  /tmp/openstego.deb" \
        | sha256sum -c - \
    && apt-get install -y --no-install-recommends /tmp/openstego.deb \
    && rm -f /tmp/openstego.deb \
    && apt-get purge -y git curl build-essential \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

# Stegcore's command line binary, out of its own published image. 8.6 MB
# against an image measured in gigabytes, and the reasoning for taking it
# from there rather than building it is at the stage declaration above.
COPY --from=stegcore-source /usr/local/bin/stegcore /usr/local/bin/stegcore

# hstego's private libjpeg and its venv, from the builder stage above. Built
# against the identical Debian digest, so the shared libraries match.
COPY --from=hstego-builder /usr/local/lib/ /usr/local/lib/
COPY --from=hstego-builder /usr/local/include/ /usr/local/include/
COPY --from=hstego-builder /opt/hstego-venv /opt/hstego-venv
RUN ldconfig

# THE TWO LICENCES THAT WERE MISSING, and the one that was missing mattered
# most: our own.
#
# SOURCE-OFFER.md tells a reader the licences are in
# /usr/share/doc/<package>/copyright inside the image, "one per package". That
# is true of the 274 packages apt installed and it was false for exactly the
# two that did not come from apt: stegcore, which is ours and is the most
# demanding licence in the image, and hstego. A platform engineer checked the
# claim in one command on 2026-10-01 and falsified it, which is the worst
# shape a documentation claim can have: specific, checkable, and wrong.
#
# Debian's convention is followed rather than invented: a copyright file that
# names the holder and the licence, and defers the full text to
# /usr/share/common-licenses where the distribution already ships it. hstego's
# upstream LICENSE.txt travels in its own dist-info, so that one is copied
# verbatim and no text is retyped anywhere.
RUN set -eu; \
    mkdir -p /usr/share/doc/stegcore /usr/share/doc/hstego; \
    { \
      echo "Upstream-Name: Stegcore"; \
      echo "Source: https://github.com/The-Malware-Files/Stegcore"; \
      echo ""; \
      echo "Files: /usr/local/bin/stegcore"; \
      echo "Copyright: Daniel Iwugo"; \
      echo "License: AGPL-3.0-or-later"; \
      echo ""; \
      echo "This binary is not a Debian package and was copied in from its own"; \
      echo "published image. Its acceptable use policy is published separately at"; \
      echo "https://github.com/The-Malware-Files/Stegcore/blob/main/AUP.md and is"; \
      echo "not part of the licence grant."; \
      echo ""; \
      echo "License: AGPL-3.0-or-later"; \
      echo " The full text of the GNU Affero General Public License version 3 is in"; \
      echo " /usr/share/common-licenses/AGPL-3.0 in this image."; \
    } > /usr/share/doc/stegcore/copyright; \
    { \
      echo "Upstream-Name: hstego"; \
      echo "Source: https://github.com/daniellerch/hstego"; \
      echo ""; \
      echo "Installed from source at commit $(cat /opt/hstego-venv/HSTEGO_COMMIT), into"; \
      echo "/opt/hstego-venv, rather than as a Debian package. Two native extensions"; \
      echo "were compiled from that commit. The upstream licence text follows"; \
      echo "verbatim."; \
      echo ""; \
      cat /opt/hstego-venv/lib/python*/site-packages/hstego*.dist-info/licenses/LICENSE.txt; \
    } > /usr/share/doc/hstego/copyright; \
    # Fail the build rather than ship the claim unbacked again. Reversing the
    # earlier failure means this has to be checked here, not trusted.
    for p in stegcore hstego; do \
        test -s "/usr/share/doc/$p/copyright" \
            || { echo "no copyright file written for $p" >&2; exit 1; }; \
    done

# numba compiles hot loops with cache=True, which writes next to the module
# in site-packages; these containers run as the calling user with all
# capabilities dropped, so site-packages is not writable. Point the cache
# somewhere writable instead of running the container as root.
ENV NUMBA_CACHE_DIR=/tmp/numba-cache \
    LD_LIBRARY_PATH=/usr/local/lib \
    # A writable HOME for whatever uid runs this.
    #
    # The documented way to run this image passes --user so the files it writes
    # belong to the person who ran it rather than to root. That gives the
    # process a uid with no passwd entry and no home, and openstego then dies
    # in Files.createDirectories trying to make its preferences directory
    # before it does any work at all. /tmp is world writable, so it works for
    # every uid including root, which is why this is set here rather than left
    # to the caller to remember.
    HOME=/tmp

# The freshness banner: build date and commit, printed on first run, because
# "you cannot tell whether it is alive without trying it" is the exact
# complaint about the toolkit image this one replaces. Pass both at build time,
# or use ./build.sh, which works them out and refuses a build that cannot:
#   docker build --build-arg STEGOBENCH_BUILD_DATE="$(date -u +%Y-%m-%d)" \
#                --build-arg STEGOBENCH_VCS_REF="$(git rev-parse --short HEAD)" .
ARG STEGOBENCH_BUILD_DATE=unknown
ARG STEGOBENCH_VCS_REF=unknown
# The tag this image was built as, so the banner can tell a reader how to run
# it rather than printing a name that may not resolve. A hardcoded name sent a
# reader to a "pull access denied" error that reads as a login problem.
ARG STEGOBENCH_IMAGE=
ENV STEGOBENCH_BUILD_DATE=${STEGOBENCH_BUILD_DATE} \
    STEGOBENCH_VCS_REF=${STEGOBENCH_VCS_REF} \
    STEGOBENCH_IMAGE=${STEGOBENCH_IMAGE}
LABEL org.opencontainers.image.created=${STEGOBENCH_BUILD_DATE} \
      org.opencontainers.image.revision=${STEGOBENCH_VCS_REF}

COPY entrypoint.sh /usr/local/bin/stegobench-toolkit-entrypoint
RUN chmod +x /usr/local/bin/stegobench-toolkit-entrypoint

WORKDIR /data
ENTRYPOINT ["stegobench-toolkit-entrypoint"]
CMD []

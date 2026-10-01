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

ARG HSTEGO_REF=master
RUN python3 -m venv /opt/hstego-venv \
    && /opt/hstego-venv/bin/pip install --no-cache-dir \
        imageio scipy pycryptodome numba pillow \
    && /opt/hstego-venv/bin/pip install --no-cache-dir \
        "git+https://github.com/daniellerch/hstego@${HSTEGO_REF}"

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
      org.opencontainers.image.licenses="AGPL-3.0-or-later AND GPL-2.0-only AND GPL-3.0-only AND BSD-3-Clause AND MIT"

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
    && curl -fsSL -o /tmp/openstego.deb \
        "https://github.com/syvaidya/openstego/releases/download/openstego-${OPENSTEGO_VERSION}/openstego_${OPENSTEGO_VERSION}-1_all.deb" \
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

# numba compiles hot loops with cache=True, which writes next to the module
# in site-packages; these containers run as the calling user with all
# capabilities dropped, so site-packages is not writable. Point the cache
# somewhere writable instead of running the container as root.
ENV NUMBA_CACHE_DIR=/tmp/numba-cache \
    LD_LIBRARY_PATH=/usr/local/lib

# The freshness banner the constellation plan (06-docker-image.md section 5)
# asks for: build date and commit, printed on first run, because "you cannot
# tell whether it is alive without trying it" is the exact complaint about
# the toolkit image this one replaces. Pass both at build time:
#   docker build --build-arg STEGOBENCH_BUILD_DATE="$(date -u +%Y-%m-%d)" \
#                --build-arg STEGOBENCH_VCS_REF="$(git rev-parse --short HEAD)" .
ARG STEGOBENCH_BUILD_DATE=unknown
ARG STEGOBENCH_VCS_REF=unknown
ENV STEGOBENCH_BUILD_DATE=${STEGOBENCH_BUILD_DATE} \
    STEGOBENCH_VCS_REF=${STEGOBENCH_VCS_REF}
LABEL org.opencontainers.image.created=${STEGOBENCH_BUILD_DATE} \
      org.opencontainers.image.revision=${STEGOBENCH_VCS_REF}

COPY entrypoint.sh /usr/local/bin/stegobench-toolkit-entrypoint
RUN chmod +x /usr/local/bin/stegobench-toolkit-entrypoint

WORKDIR /data
ENTRYPOINT ["stegobench-toolkit-entrypoint"]
CMD []

# The default toolkit image: core + tools tier (operator decision D3,
# 2026-09-21). Carries steghide, outguess, openstego, stegosuite, zsteg,
# stegexpose and hstego, everything measured at 118 MB to 1.41 GB, so a user
# who wants to run steghide is not made to download a deep learning stack
# first. See ../IMAGES.md for how each of these numbers was measured and
# ./README.md for how to add Aletheia on top once you need it.
#
# UNBUILT AND UNTESTED. Authored from the seven per-tool Dockerfiles in this
# repository, which build and run individually; this file has not itself been
# built. Building is heavy work that belongs on a dedicated machine, not this
# one, per the working constraints for this change. Build and smoke-test it
# before it is ever published.
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
      org.opencontainers.image.description="steghide, outguess, openstego, stegosuite, zsteg, stegexpose, hstego in one image" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later"

# git and curl are build-time only: git clones StegExpose's source (it ships
# as source, not a release jar) and curl fetches OpenStego's .deb. Both are
# purged after use, matching the pattern the per-tool Dockerfiles already use
# for the same reason (see tools/stegexpose/Dockerfile, tools/zsteg/Dockerfile).
ARG STEGEXPOSE_REF=master
ARG OPENSTEGO_VERSION=0.8.6
ARG ZSTEG_VERSION=0.2.13

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        # the three tools that are plain Debian packages
        steghide outguess stegosuite \
        # a headless JRE for openstego and StegExpose, both Java
        openjdk-17-jre-headless \
        # zsteg is pure Ruby; `file` is a runtime dependency (zsteg shells
        # out to libmagic to identify the carrier, per tools/zsteg/Dockerfile)
        ruby file \
        # build-time only, purged below
        git curl ca-certificates build-essential \
        # runtime libs numba/scipy wheels commonly need beyond what a
        # manylinux wheel bundles itself
        libgomp1 \
        python3 \
    && gem install zsteg -v "${ZSTEG_VERSION}" --no-document \
    && git clone https://github.com/b3dk7/StegExpose /opt/stegexpose \
    && (cd /opt/stegexpose && git checkout "${STEGEXPOSE_REF}") \
    && curl -fsSL -o /tmp/openstego.deb \
        "https://github.com/syvaidya/openstego/releases/download/openstego-${OPENSTEGO_VERSION}/openstego_${OPENSTEGO_VERSION}-1_all.deb" \
    && apt-get install -y --no-install-recommends /tmp/openstego.deb \
    && rm -f /tmp/openstego.deb \
    && apt-get purge -y git curl build-essential \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

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

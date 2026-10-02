# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# THE HARNESS IN A CONTAINER, WHICH IS NOT THE SAME THING AS THE PLUGINS
#
# This builds ONE image: `stegobench` itself, so somebody can run the tool
# without installing a Rust toolchain. It has nothing to do with the nine
# detector and embedder images the registry pins by digest. Those are other
# people's tools, built elsewhere, and this file neither contains nor fetches
# them. The repository has been loose about that distinction before, so it is
# worth stating in the first paragraph of the file that creates the confusion.
#
# WHAT THE IMAGE NEEDS AT RUNTIME, WHICH IS ALMOST NOTHING
#
# Measured, not assumed: `crates/stegobench-cli/build.rs` compiles the registry
# TOMLs, the self-test fixtures, the host adapters and the starter corpus INTO
# the binary. So the shipped layer carries no data files, and `doctor`, `list`,
# `describe` and `score` all work from a directory with no checkout in it.
# Those four trees are build-stage inputs; see `.dockerignore`.
#
# Two outside programs are reached at run time and neither is the binary's
# business to provide:
#
#   curl    `fetch` shells out to it, with every deadline and protocol
#           restriction on the command line. Installed below, because a
#           `fetch` that cannot run is a command missing from the image.
#   docker  how a containerised detector is run. DELIBERATELY NOT INSTALLED,
#           see the next paragraph.
#
# WHY THERE IS NO DOCKER CLIENT IN HERE
#
# Putting one in would only help somebody who then mounts the host's docker
# socket into this container, and that mount is root on the host: a process
# that can talk to the socket can start a privileged container with the host
# filesystem bound into it. This project will not ship an image whose
# documented usage is a host-root escalation. `doctor` inside this container
# therefore reports every containerised tool as having no container runtime,
# which is the honest answer and is what `docs/guide/running-in-a-container.md`
# says it will say. To score with a containerised detector, install the harness
# on the host.

# ── Stage one: build ────────────────────────────────────────────────────────
#
# rust:1.98.1-alpine, pinned by the digest of its multi-architecture index so
# the same reference resolves on amd64 and arm64. The tag is kept in the
# comment because a digest alone tells a reader nothing about what it is.
#
# Why this image: its rustc is 1.98.1, which is exactly what
# `rust-toolchain.toml` pins, so the build needs no toolchain download and
# cannot drift from what CI compiles. Its default target is musl, and musl
# links statically by default, so the binary that comes out of this stage
# depends on no shared library at all. It is an Official Image, maintained by
# the Rust project's own release tooling through docker-library.
FROM rust:1.98.1-alpine@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS build

WORKDIR /src
COPY . .

# `--locked` so the committed Cargo.lock decides every version, which is the
# same rule the rest of this project builds under. A build that resolves
# whatever is newest is a build whose output nobody can reproduce.
#
# No `--offline`: the lockfile pins the versions and this still has to download
# them once. Nothing here is fetched by a mutable reference.
RUN cargo build --release --locked -p stegobench-cli

# Proof, inside the build, that the thing being shipped links no shared
# library. The runtime stage below is Alpine and carries a musl of its own, so
# nothing breaks today if this stops being true; the check is here for the next
# person who tries a smaller base, who would otherwise find out from a user
# reporting "no such file or directory" for a binary that is plainly there,
# which is what a missing loader looks like.
#
# The test is on DT_NEEDED entries rather than on `ldd`. Rust's musl target
# produces a static position independent executable, and `ldd` prints
# `/lib/ld-musl-x86_64.so.1` for one of those because it is its own loader, so
# reading ldd's output calls a correctly static binary dynamic.
RUN set -eu; \
    if readelf -d target/release/stegobench 2>/dev/null | grep -q 'NEEDED'; then \
        echo "stegobench links a shared library, so a smaller base would need a libc:" >&2; \
        readelf -d target/release/stegobench >&2; \
        exit 1; \
    fi; \
    ./target/release/stegobench --version

# ── Stage two: what ships ───────────────────────────────────────────────────
#
# alpine:3.24.2, pinned by its index digest, 8 platforms. Alpine rather than
# `scratch` or a distroless base for one reason: `fetch` needs curl, and a base
# with no package manager cannot have one. Everything else in here would work
# on `scratch`.
FROM alpine:3.24.2@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6

# THE ONE INPUT IN THIS FILE THAT IS NOT PINNED, AND WHY IT IS LEFT THAT WAY
#
# `apk add` resolves against the Alpine mirror at build time, so two builds of
# this file a month apart can install different curl patch releases. Pinning
# exact apk versions reads stricter and is worse: Alpine drops a package
# version from the mirror as soon as it is superseded, so a pinned line stops
# building rather than staying reproducible, and a release path that breaks on
# somebody else's schedule gets edited in a hurry.
#
# What protects a consumer is the digest of the image this produces, which is
# what the publishing workflow records and what anybody pulling it should name.
# The project's rule is that a RESULT names exact bytes; it is honoured by
# pinning the output, not by pretending a mirror is immutable.
RUN apk add --no-cache ca-certificates curl

COPY --from=build /src/target/release/stegobench /usr/local/bin/stegobench

# The man pages are generated by build.rs from the same command tree the binary
# parses against, so they cannot describe a flag it does not have. Alpine ships
# no man reader, so these sit here for a user who installs one or copies them
# out; they are about 100 KB and cannot drift, which is the whole argument for
# carrying them.
COPY --from=build /src/target/man/ /usr/share/man/man1/

COPY --from=build /src/LICENSE /src/README.md /src/CHANGELOG.md /usr/share/doc/stegobench/

# A fixed numeric uid, not a name. A corpus is bind mounted into this container
# from the host, and host permissions are decided by the number: a reader who
# has to make their files readable needs to know which uid to grant, and
# `stegobench` is not an answer they can use in `chown`. 65532 is the
# convention distroless uses for its unprivileged user, so it collides with
# nothing on a normal host.
RUN addgroup -g 65532 -S stegobench \
 && adduser -u 65532 -S -G stegobench -h /home/stegobench stegobench \
 && mkdir -p /work \
 && chown stegobench:stegobench /work
USER 65532:65532

# Where a corpus is expected to be mounted, so the documented command is
# `-v "$PWD:/work"` and relative paths behave the way they do on a host.
WORKDIR /work

# Filled in by the publishing workflow from the tag. Left as "unknown" rather
# than as a version number nobody has released, so a locally built image never
# claims to be a release.
ARG VERSION=unknown
ARG REVISION=unknown
LABEL org.opencontainers.image.title="stegobench" \
      org.opencontainers.image.description="A reproducible benchmark for steganalysis" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.source="https://github.com/elementmerc/stegobench" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"

# The binary, not a shell. `docker run <image> doctor` reads the way the
# command does on a host, and a bare `docker run <image>` prints the same short
# orientation `stegobench` with no arguments prints.
ENTRYPOINT ["/usr/local/bin/stegobench"]

# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# Build with:  tox -e package
#
# Three stages, because the analysis binary is Rust and the plugin is Python
# and neither build environment belongs in the image that ships.
#
# EVERY base is pinned by digest, not by tag. A tag can be moved under you; a
# digest is the content. The tag is kept beside each one as a comment so a
# human can see what the digest was when it was taken, which a bare digest
# does not tell you. Resolved 2026-09-19 from registry-1.docker.io.

###############################################################################
# Stage 1: build the analysis binary
#
# STEGCORE_COMMIT is a commit, and this used to claim that while holding a tag
# object: 22d05afa is the annotated v4.1.0 tag, which dereferences to
# d179ddfd. Both are content addressed so either builds reproducibly, but an
# examiner asked which code produced a finding deserves the answer to be the
# thing the variable is named after.
#
# That commit predates `rust-toolchain.toml`, which arrived later on dev, so
# there is no upstream declaration to match the builder against. The version
# below is this image's choice and nothing upstream confirms it. An earlier
# comment here asserted the opposite.
FROM rust@sha256:ae1a730a949f727611a5c684e1e26e5a9bb9885b34f65a442744ca8a61c86ca5 AS stegcore
# rust:1.98.1-bookworm
ARG STEGCORE_REPO=https://github.com/The-Malware-Files/Stegcore.git
ARG STEGCORE_COMMIT=d179ddfd6ba6e00310d7b9e6859dec89bf8460ec
RUN git clone --filter=blob:none "$STEGCORE_REPO" /src \
 && cd /src \
 && git checkout --detach "$STEGCORE_COMMIT" \
 && cargo build --release -p stegcore-cli \
 && install -D target/release/stegcore /out/stegcore \
 && /out/stegcore --version

###############################################################################
# Stage 2: resolve the Python dependencies
FROM python@sha256:6faba2c56370992b0456e11f781f00a09b7ecc3c4e97ef3e39163d01a94aea8d AS builder
# python:3.13
ARG PIP_INDEX_URL=https://pypi.org/simple/
RUN python -m venv /venv
ENV PATH="/venv/bin:$PATH"
COPY requirements.txt /requirements.txt
RUN pip install -Ur /requirements.txt

###############################################################################
# Stage 3: the plugin image
FROM python@sha256:9d2e5553305c7c7b0097999bb17187c69b921ccd6bc9d40e4bb5ebe652c00285
# python:3.13-slim
COPY --from=builder /venv /venv
# 0755 explicitly. A COPY that lands without the execute bit raises
# PermissionError rather than FileNotFoundError, which is a different branch
# and used to escape the plugin's handler and kill the whole trace.
COPY --from=stegcore --chmod=0755 /out/stegcore /opt/stegcore/stegcore
ENV PATH="/venv/bin:$PATH"
ENV STEGCORE_BINARY=/opt/stegcore/stegcore

COPY plugin.py stegcore_adapter.py trailing_data.py /app/

# Fail the build rather than the first exhibit.
RUN /opt/stegcore/stegcore --version

EXPOSE 8999
ENTRYPOINT ["serve_plugin", "-v"]
CMD ["/app/plugin.py", "8999"]

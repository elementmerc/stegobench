# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
#
# Build with:  tox -e package
#
# Three stages, because the analysis binary is Rust and the plugin is Python
# and neither build environment belongs in the image that ships.

###############################################################################
# Stage 1: build the analysis binary
#
# Pinned to a commit rather than a tag. A tag can be moved; a commit cannot,
# and an investigator has to be able to say which code produced a finding.
# The tag matches `rust-toolchain.toml` in the pinned commit. If they drift,
# rustup silently downloads the pinned toolchain at build time and the image
# carries two, so this is kept in step deliberately.
FROM rust:1.98.1-bookworm AS stegcore
ARG STEGCORE_REPO=https://github.com/The-Malware-Files/Stegcore.git
ARG STEGCORE_COMMIT=22d05afa7db4b96e7c1652133a4d8b509975a0f2
RUN git clone --filter=blob:none "$STEGCORE_REPO" /src \
 && cd /src \
 && git checkout --detach "$STEGCORE_COMMIT" \
 && cargo build --release -p stegcore-cli \
 && install -D target/release/stegcore /out/stegcore \
 && /out/stegcore --version

###############################################################################
# Stage 2: resolve the Python dependencies
FROM python:3.13 AS builder
ARG PIP_INDEX_URL=https://pypi.org/simple/
RUN python -m venv /venv
ENV PATH="/venv/bin:$PATH"
COPY requirements.txt /requirements.txt
RUN pip install -Ur /requirements.txt

###############################################################################
# Stage 3: the plugin image
FROM python:3.13-slim
COPY --from=builder /venv /venv
COPY --from=stegcore /out/stegcore /opt/stegcore/stegcore
ENV PATH="/venv/bin:$PATH"
ENV STEGCORE_BINARY=/opt/stegcore/stegcore

COPY plugin.py stegcore_adapter.py trailing_data.py /app/

EXPOSE 8999
ENTRYPOINT ["serve_plugin", "-v"]
CMD ["/app/plugin.py", "8999"]

# zsteg (zed-0xff) — PNG/BMP bit-plane steganalysis (detector).
# Base pinned by digest on 2026-09-21. A tag moves; a digest does not,
# and a corpus whose tools were built from "whatever stable-slim meant
# that week" is not rebuildable in any sense a reader can check.
FROM ruby@sha256:53012760b6eee24268541775b379a96578301ba0ade9b8d60d39cb50360a4cf7
ARG ZSTEG_VERSION=0.2.13
# `file` (libmagic) is a runtime dependency: zsteg shells out to it to identify
# the carrier, and without it every run aborts with a nil-stdin error. It is
# installed manually so the later autoremove leaves it in place.
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential file \
    && gem install zsteg -v "${ZSTEG_VERSION}" --no-document \
    && apt-get purge -y build-essential \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /data
# zsteg -a stego.png        (try all bit-planes / orders)
ENTRYPOINT ["zsteg"]
CMD ["--help"]

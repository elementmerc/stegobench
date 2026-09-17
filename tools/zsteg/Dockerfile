# zsteg (zed-0xff) — PNG/BMP bit-plane steganalysis (detector).
FROM ruby:3.3-slim

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

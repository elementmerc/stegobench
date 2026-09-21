# stegoVeritas (bannsec) — multi-technique image steganalysis (detector).
# Pulls in exiftool, steghide, foremost etc. via its dep installer.
# Base pinned by digest on 2026-09-21. A tag moves; a digest does not,
# and a corpus whose tools were built from "whatever stable-slim meant
# that week" is not rebuildable in any sense a reader can check.
FROM python@sha256:174bec68e0451bffabbb08c7d5d21c6b253f772d81d52b9558af97bb3159b761
ARG STEGOVERITAS_VERSION=2.0.0
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       exiftool steghide foremost fcrackzip \
    && pip install --no-cache-dir "stegoveritas==${STEGOVERITAS_VERSION}" \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /data
# stegoveritas /data/stego.png   (writes results under ./results)
ENTRYPOINT ["stegoveritas"]
CMD ["--help"]

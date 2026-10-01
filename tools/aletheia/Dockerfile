# Aletheia (daniellerch) — the steganalysis parity reference (CLAUDE.md A3).
# Detector. We benchmark Stegcore's SPA/RS/WS against Aletheia's
# implementations to floating-point precision on a documented corpus.
# Base pinned by digest on 2026-09-21. A tag moves; a digest does not,
# and a corpus whose tools were built from "whatever stable-slim meant
# that week" is not rebuildable in any sense a reader can check.
FROM python@sha256:174bec68e0451bffabbb08c7d5d21c6b253f772d81d52b9558af97bb3159b761
# Pinned to a commit for reproducible numbers (fairness requirement). Override
# with --build-arg ALETHEIA_REF=<sha>.
#
# This default said "tracks master; PIN before any published benchmark", which
# put the obligation on whoever ran the build and nowhere a reader could check
# it. The commit below is not a fresh choice: it is what `stegobench/aletheia:pinned`
# and `stegobench/aletheia-rich:pinned` were actually built from, read out of
# /opt/aletheia/.git in those images on 2026-10-01, so pinning it reproduces
# the images that were measured at 8.34 GB and 9.09 GB rather than selecting
# new ones.
ARG ALETHEIA_REF=1baf974ea8fcf0b51802935d9acbe59903d06845

# libmagic1 backs the python-magic module that aletheialib.attacks imports.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       git build-essential libjpeg-dev zlib1g-dev libmagic1 \
    && rm -rf /var/lib/apt/lists/*

RUN git clone https://github.com/daniellerch/aletheia /opt/aletheia \
    && cd /opt/aletheia \
    && git checkout "${ALETHEIA_REF}" \
    && git rev-parse HEAD > /opt/aletheia/.pinned-commit

WORKDIR /opt/aletheia
# Classical detectors (spa/rs/ws) need the numerical stack, not the ML
# extras; install the lean set to keep the build reliable over flaky nets.
# pandas is imported unconditionally at the top of aletheia.py; python-magic is
# imported by aletheialib.attacks, which the classical spa/rs/ws detectors use.
RUN pip install --no-cache-dir numpy scipy imageio scikit-image pillow pandas python-magic

WORKDIR /data
# python3 /opt/aletheia/aletheia.py spa  /data/stego.png
# python3 /opt/aletheia/aletheia.py rs   /data/stego.png
# python3 /opt/aletheia/aletheia.py ws   /data/stego.png
ENTRYPOINT ["python3", "/opt/aletheia/aletheia.py"]
CMD []

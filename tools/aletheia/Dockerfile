# Aletheia (daniellerch) — the steganalysis parity reference (CLAUDE.md A3).
# Detector. We benchmark Stegcore's SPA/RS/WS against Aletheia's
# implementations to floating-point precision on a documented corpus.
FROM python:3.11-slim

# Pin to a commit for reproducible numbers (fairness requirement). Override
# with --build-arg ALETHEIA_REF=<sha>. Default tracks master; PIN before any
# published benchmark so the comparison is stable across releases.
ARG ALETHEIA_REF=master

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

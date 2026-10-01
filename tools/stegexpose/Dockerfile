# StegExpose (b3dk7) — LSB steganalysis ensemble (Sample Pairs, RS, Chi-square,
# Primary Sets) for lossless images. Detector. Distributed as a jar.
# Base pinned by digest on 2026-09-21. A tag moves; a digest does not,
# and a corpus whose tools were built from "whatever stable-slim meant
# that week" is not rebuildable in any sense a reader can check.
FROM eclipse-temurin@sha256:24cd8eed18b5976441d27b45823490eb5e8efff4b3ecdc632e442717ea66f160
# Pinned rather than tracking master. The built image does not retain its .git,
# so this was not read back from it; upstream is archived and its last commit is
# e8312cb from 2018-08-14, so there is no other candidate. Written down as
# unverified against the built image for the same reason as the aletheia-rich
# pin beside it.
ARG STEGEXPOSE_REF=e8312cb0973d2a97deffc771d8e5910df8494698
RUN apt-get update \
    && apt-get install -y --no-install-recommends git \
    && git clone https://github.com/b3dk7/StegExpose /opt/stegexpose \
    && cd /opt/stegexpose \
    && git checkout "${STEGEXPOSE_REF}" \
    && apt-get purge -y git && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /data
# StegExpose scans a DIRECTORY of images and prints a CSV verdict per file:
#   java -jar /opt/stegexpose/StegExpose.jar /data
ENTRYPOINT ["java", "-jar", "/opt/stegexpose/StegExpose.jar"]
CMD []

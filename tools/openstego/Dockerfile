# OpenStego (Syvaidya) — Java LSB steganography tool.
# Embedder + extractor. Null-LSB mode (no password) is the ground truth
# for Stegcore's v4.1 OpenStego fingerprint.
# Base pinned by digest on 2026-09-21. A tag moves; a digest does not,
# and a corpus whose tools were built from "whatever stable-slim meant
# that week" is not rebuildable in any sense a reader can check.
FROM eclipse-temurin@sha256:24cd8eed18b5976441d27b45823490eb5e8efff4b3ecdc632e442717ea66f160
ARG OPENSTEGO_VERSION=0.8.6
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates \
    && curl -fsSL -o /tmp/openstego.deb \
       "https://github.com/syvaidya/openstego/releases/download/openstego-${OPENSTEGO_VERSION}/openstego_${OPENSTEGO_VERSION}-1_all.deb" \
    && apt-get install -y --no-install-recommends /tmp/openstego.deb \
    && rm -f /tmp/openstego.deb \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /data
# openstego embed -mf <msg> -cf <cover> -sf <stego>   (Null-LSB when no -p)
# openstego extract -sf <stego> -xf <out>
ENTRYPOINT ["openstego"]
CMD ["--help"]

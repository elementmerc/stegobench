# OutGuess — JPEG steganography with statistical-correction embedding.
# Embedder + extractor. Distro package is version-stable.
# Base pinned by digest on 2026-09-21. A tag moves; a digest does not,
# and a corpus whose tools were built from "whatever stable-slim meant
# that week" is not rebuildable in any sense a reader can check.
FROM debian@sha256:520c7157b4b1c3c46fa56bac0e748ea21d60956c0512b300e3b16d8757f68e2d
RUN apt-get update \
    && apt-get install -y --no-install-recommends outguess \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /data
# outguess -k PASS -d secret.txt cover.jpg stego.jpg
# outguess -k PASS -r stego.jpg out.txt
ENTRYPOINT ["outguess"]
CMD ["-h"]

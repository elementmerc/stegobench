# Stegosuite — Java, AES encrypted, LSB spread across the image.
#
# Here because it is the tool a desktop user is most likely to find already
# packaged for their distribution. Not a research implementation: what somebody
# who is not a researcher actually ends up running.
# Base pinned by digest on 2026-09-21. A tag moves; a digest does not,
# and a corpus whose tools were built from "whatever stable-slim meant
# that week" is not rebuildable in any sense a reader can check.
FROM debian@sha256:520c7157b4b1c3c46fa56bac0e748ea21d60956c0512b300e3b16d8757f68e2d
RUN apt-get update \
    && apt-get install -y --no-install-recommends stegosuite \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /data
# stegosuite embed -f <payload> -k <password> <cover>
# writes <cover-stem>_embed.<ext> beside the cover rather than to a chosen path
ENTRYPOINT ["stegosuite"]
CMD ["--help"]

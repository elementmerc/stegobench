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
# stegosuite embed -f <payload> -k <password> -o <stego> <cover>
#
# EMBED can be told where to write, with -o/--output. This comment said it could
# not, which is worse than saying nothing: it was read and believed, and it cost
# somebody time before they measured the tool rather than trusting the file.
# Without -o it writes <cover-stem>_embed.<ext> beside the cover.
#
# EXTRACT cannot. It has no output option at all and writes the payload's
# ORIGINAL EMBEDDED FILENAME into the working directory, so it will overwrite a
# file of that name with no prompt and exit 0. Extract into an empty directory.
#
# It also needs an RGB cover. Its LSB method spreads across colour channels, so
# on a greyscale PNG it cannot read back what it wrote and fails with
# SteganoEncryptionException on its own output.
ENTRYPOINT ["stegosuite"]
CMD ["--help"]

# Stegosuite — Java, AES encrypted, LSB spread across the image.
#
# Here because it is the tool a desktop user is most likely to find already
# packaged for their distribution. Not a research implementation: what somebody
# who is not a researcher actually ends up running.
FROM debian:stable-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends stegosuite \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /data
# stegosuite embed -f <payload> -k <password> <cover>
# writes <cover-stem>_embed.<ext> beside the cover rather than to a chosen path
ENTRYPOINT ["stegosuite"]
CMD ["--help"]

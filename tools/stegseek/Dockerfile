# stegseek (RickdeJager), a steghide passphrase cracker, several thousand
# times faster than stegbrute/stegcracker because it builds a dictionary
# lookup rather than shelling out to steghide once per candidate word.
# The most requested missing tool per the v0.1 shortlist (item V12): the
# first thing a CTF reader looks for and the one classic tool this project
# did not already carry.
#
# Base pinned by digest on 2026-09-21, the same Debian digest every other
# Debian-based tool image here uses. A tag moves; a digest does not, and a
# corpus whose tools were built from "whatever stable-slim meant that week"
# is not rebuildable in any sense a reader can check.
#
# UNBUILT AND UNTESTED. Authored, not built: building is heavy work kept off
# this machine for this change. Build and smoke-test before trusting it.
FROM debian@sha256:520c7157b4b1c3c46fa56bac0e748ea21d60956c0512b300e3b16d8757f68e2d

ARG STEGSEEK_VERSION=0.6
# Upstream ships a Debian package on its GitHub releases rather than an apt
# repository, the same distribution shape openstego uses in this project's
# tools/openstego/Dockerfile, so the same curl-then-install pattern applies.
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl ca-certificates \
    && curl -fsSL -o /tmp/stegseek.deb \
       "https://github.com/RickdeJager/stegseek/releases/download/v${STEGSEEK_VERSION}/stegseek_${STEGSEEK_VERSION}-1.deb" \
    && apt-get install -y --no-install-recommends /tmp/stegseek.deb \
    && rm -f /tmp/stegseek.deb \
    && apt-get purge -y curl \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /data
# stegseek <stegofile> <wordlist>            crack against a wordlist
# stegseek --crack -sf stego.jpg -wl rockyou.txt
# A wordlist is not bundled: rockyou.txt alone is larger than every other
# tool in this registry combined, and shipping it would turn "pull the
# cracker" into "pull a password list", which is a different decision for a
# user to make on purpose rather than one this image should make for them.
ENTRYPOINT ["stegseek"]
CMD ["--help"]

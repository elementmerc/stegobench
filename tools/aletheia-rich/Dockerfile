# Aletheia with the rich-model feature extractors working.
#
# WHY THIS IS A SECOND IMAGE RATHER THAN A CHANGE TO THE FIRST
# -----------------------------------------------------------
# The lean `aletheia` image runs the structural detectors (spa, rs, ws, triples)
# on a Python-only stack, builds in about a minute, and is what the classical
# panel uses. This one exists because the rich-model feature extractors do not
# run on that stack at all.
#
# SRM, SRMQ1, DCTR and GFR are not implemented in Python. Aletheia drives them
# through Octave (`aletheialib/octave_interface.py:22`, `M_BIN="octave -q ..."`),
# so the feature extraction this evaluation needs pulls in a whole numerical
# environment that the classical detectors have no use for. Keeping them apart
# means the panel's build stays fast and reliable, and a failure in the heavy
# image cannot take the cheap detectors down with it.
#
# THE RUNTIME DOWNLOAD, AND WHY IT IS STAGED AT BUILD TIME
# -------------------------------------------------------
# Aletheia does not ship the Octave code. It fetches each `.m` file on first use
# from `aletheia-external-resources`, prints that file's licence, and waits at a
# `y/n` prompt for a human to accept it (`aletheialib/utils.py:140`).
#
# Our detector containers run with no network and no terminal, so that path
# cannot complete: it would hang on a prompt nobody can answer, behind a
# download that cannot happen. The same function returns early when the file is
# already present (`utils.py:153`), so staging the whole cache during the build
# is the supported way through rather than a workaround.
#
# This makes the image reproducible in the way a benchmark needs. Nothing is
# fetched at run time, so a result does not depend on a GitHub repository being
# reachable, or unchanged, on the day somebody re-runs it.
#
# ON THE LICENCES OF THE STAGED CODE
# ----------------------------------
# Each `.m` carries its own licence from its original authors, and they are
# research codes with varying terms. They are cloned here and used locally to
# extract features. **This image is not published and these files are not
# redistributed.** The licences come with them, in the same directory, so
# anybody inspecting the image can read the terms that apply to what they are
# running.
FROM python:3.11-slim

ARG ALETHEIA_REF=df4fc2e51352ddafeafd033d0f77efd28ecb8c77
# Pinned by commit, not by branch, because a benchmark that moves under you is
# not a benchmark. This is the commit the classical image resolved to on
# 2026-09-17.

ARG RESOURCES_REF=main

# Just `octave`. The obvious spellings `octave-cli` and `liboctave-dev` are
# Ubuntu and Fedora names and do not exist in Debian trixie, which is what this
# base image is; asking for them fails the whole apt step rather than being
# ignored, which is how this was found.
#
# The consequence, stated rather than discovered later: without mkoctfile the
# JPEG toolbox mex cannot be built, so the JPEG-domain extractors (DCTR, GFR)
# may not run in this image. SRM is the one this evaluation turns on and it
# works on pixels, so it needs no mex at all. The build below asserts SRM
# specifically for that reason.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       git build-essential libjpeg-dev zlib1g-dev libmagic1 \
       octave \
    && rm -rf /var/lib/apt/lists/*

RUN git clone https://github.com/daniellerch/aletheia /opt/aletheia \
    && cd /opt/aletheia \
    && git checkout "${ALETHEIA_REF}" \
    && git rev-parse HEAD > /opt/aletheia/.pinned-commit

# hdf5storage is what Ensemble4Stego writes its feature matrices with, and
# scikit-learn is imported by the e4s entry point. Both are absent from the lean
# image on purpose. We train with our own classifier rather than e4s (see
# generators/fld_ensemble.py for why), but installing them keeps `e4s` available
# as a cross-check rather than silently missing.
RUN pip install --no-cache-dir \
      numpy scipy imageio scikit-image pillow pandas python-magic \
      hdf5storage scikit-learn

# Stage the Octave code into the cache Aletheia looks in, so no download and no
# licence prompt is ever reached at run time. Record the resources commit beside
# it: without that, "which SRM did this number come from" has no answer.
RUN git clone --depth 1 --branch "${RESOURCES_REF}" \
      https://github.com/daniellerch/aletheia-external-resources /tmp/aer \
    && mkdir -p /opt/aletheia/aletheia-cache \
    && cp -r /tmp/aer/octave /opt/aletheia/aletheia-cache/octave \
    && cp -r /tmp/aer/octave/jpeg_toolbox /opt/aletheia/aletheia-cache/jpeg_toolbox 2>/dev/null || true \
    && git -C /tmp/aer rev-parse HEAD > /opt/aletheia/.pinned-resources \
    && rm -rf /tmp/aer

# The cache layout Aletheia expects is aletheia-cache/octave/<METHOD>.m, and the
# clone nests them one level deeper under octave/code/. Flatten rather than
# assume, and fail loudly here at build time if SRM.m did not land, because the
# alternative is discovering it mid-run through an interactive prompt.
RUN if [ -d /opt/aletheia/aletheia-cache/octave/code ]; then \
      cp /opt/aletheia/aletheia-cache/octave/code/* /opt/aletheia/aletheia-cache/octave/ ; \
    fi \
    && test -f /opt/aletheia/aletheia-cache/octave/SRM.m \
    && test -f /opt/aletheia/aletheia-cache/octave/DCTR.m \
    && test -f /opt/aletheia/aletheia-cache/octave/ensemble_training.m \
    && octave -q --no-gui --eval "disp('octave ok')" \
    && echo "staged: SRM, DCTR, ensemble_training"

WORKDIR /data
ENTRYPOINT ["python3", "/opt/aletheia/aletheia.py"]
CMD []

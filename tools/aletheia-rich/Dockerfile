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
# Built ON TOP of the lean image rather than from the base, because the Aletheia
# clone is the slowest step by a wide margin (over thirty minutes from this
# build box, twice) and the lean image already has it, at the same pinned
# commit. Layering also guarantees the two images cannot drift to different
# revisions of the detector, which would make the classical panel and the rich
# baseline incomparable in a way nothing would report.
FROM stegobench/aletheia:pinned

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
#
# The three Octave Forge packages are not optional and are not part of `octave`.
# `octave_interface.py:169` emits `pkg load image; pkg load signal; pkg load nan`
# into every extraction, and without them Octave exits before writing its output.
# Aletheia then reports "Cannot extract feactures from <file>" and carries on to
# the next image, so a run against a whole corpus completes, produces nothing,
# and never returns a non-zero status. That is the failure mode this image is
# built to make impossible, and it is why the smoke test at the end exists.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       git build-essential libjpeg-dev zlib1g-dev libmagic1 \
       octave octave-image octave-signal octave-nan \
    && rm -rf /var/lib/apt/lists/*

# No clone: /opt/aletheia comes from the base image, at the commit recorded in
# its own .pinned-commit. Assert it is there rather than assume, because a
# rebuild against a differently named base would otherwise fail much later and
# much less clearly.
RUN test -f /opt/aletheia/aletheia.py \
    && echo "inherited aletheia at $(cat /opt/aletheia/.pinned-commit)"

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

# Extract SRM from one real image at build time and refuse the image if it does
# not produce a feature vector.
#
# This is here because the alternative was measured. Aletheia catches a failed
# extraction per image, prints "Cannot extract feactures", and continues, so a
# broken image still exits zero on a corpus of any size and writes an empty
# feature file. The first run against 2,000 images looked like it worked and had
# produced nothing, and the cause was three missing Octave packages rather than
# anything about the corpus. A build-time smoke test turns that into a failure
# at the only point where it is cheap to read.
RUN python3 -c "import numpy; from PIL import Image; \
      Image.fromarray((numpy.random.default_rng(0).integers(0,256,(256,256))).astype('uint8'), 'L').save('/tmp/smoke.png')" \
    && python3 /opt/aletheia/aletheia.py srm /tmp/smoke.png /tmp/smoke.fea \
    && test -s /tmp/smoke.fea \
    && python3 -c "import numpy,sys; \
       f=numpy.loadtxt('/tmp/smoke.fea'); \
       print('SRM smoke test:', f.shape, 'features'); \
       sys.exit(0 if f.size > 1000 else 1)" \
    && rm -f /tmp/smoke.png /tmp/smoke.fea /tmp/smoke.fea.label

WORKDIR /data
ENTRYPOINT ["python3", "/opt/aletheia/aletheia.py"]
CMD []

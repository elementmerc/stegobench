# hstego (Daniel Lerch) — HILL and J-UNIWARD with a REAL syndrome-trellis coder.
#
# This is the arm that measures what the adaptive literature assumes. `conseal`,
# which the rest of our adaptive arm uses, SIMULATES embedding at the optimal
# coding rate: it produces the change map a perfect coder would produce. hstego
# pays the real coding overhead with an actual STC. Running both over the same
# covers at the same rates measures a gap the field takes on faith.
#
# WHY libjpeg IS BUILT FROM SOURCE HERE
# ------------------------------------
# hstego's JPEG extension reads `block_size`, `min_DCT_h_scaled_size` and
# `DCT_h_scaled_size` off the libjpeg structs. Those fields exist in the
# Independent JPEG Group's libjpeg from v8 onward and NOT in libjpeg-turbo's
# jpeg62 ABI, which is what every mainstream distribution ships. Installing
# `libjpeg-dev` therefore fails to compile with "has no member named
# block_size", which is not a missing package but a different library wearing
# the same name.
#
# So the IJG release is built and installed alongside, and the extension is
# pointed at it. This is also why the tool is containerised rather than put in
# the virtual environment: it wants a libjpeg the host should not have.
FROM python:3.11-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        gcc g++ libc6-dev make curl ca-certificates git \
        tk \
    && rm -rf /var/lib/apt/lists/*

# 9e is the current IJG release. Pinned, because the struct layout this tool
# depends on is exactly the thing that changes between major versions.
ARG LIBJPEG_VERSION=9e
RUN curl -fsSL "https://www.ijg.org/files/jpegsrc.v${LIBJPEG_VERSION}.tar.gz" \
        -o /tmp/jpegsrc.tar.gz \
    && tar -xzf /tmp/jpegsrc.tar.gz -C /tmp \
    && cd "/tmp/jpeg-${LIBJPEG_VERSION}" \
    && ./configure --prefix=/usr/local --enable-shared \
    && make -j"$(nproc)" && make install \
    # `make install` ships only the public headers. hstego's extension includes
    # <jpegint.h>, which is libjpeg's INTERNAL header describing the compressor
    # and decompressor structs, and reaching into those internals is the whole
    # reason this tool can read DCT coefficients at all. So the internal headers
    # are installed by hand, before the source tree is removed.
    && cp -n /tmp/jpeg-${LIBJPEG_VERSION}/*.h /usr/local/include/ \
    && ldconfig \
    && rm -rf /tmp/jpeg*

ENV CFLAGS="-I/usr/local/include" LDFLAGS="-L/usr/local/lib" \
    LD_LIBRARY_PATH="/usr/local/lib"

ARG HSTEGO_REF=master
# hstego declares almost none of its runtime dependencies. A clean install
# produces a tool that cannot start, and the failures arrive one import at a
# time: imageio, then scipy, then pycryptodome, then numba, then Pillow, and
# finally libtk because hstego.py imports ImageTk at module scope even when it
# is being used from the command line. Tk is a GUI toolkit and has no business
# in a headless container; it is here because an unused import at module scope
# is still an import.
#
# All of this is upstream packaging rather than a preference of ours, and worth
# reporting: we already have a pull request open against the same author's
# Aletheia, so there is a channel for it.
RUN pip install --no-cache-dir imageio scipy pycryptodome numba pillow \
    && pip install --no-cache-dir "git+https://github.com/daniellerch/hstego@${HSTEGO_REF}"

# numba compiles hstegolib's hot loops with cache=True, which writes next to the
# module in site-packages. We run these containers as the calling user with all
# capabilities dropped, so site-packages is not writable and the import dies
# before the tool starts. Point the cache somewhere writable instead of running
# the container as root: the sandbox is the point.
ENV NUMBA_CACHE_DIR=/tmp/numba-cache

WORKDIR /data
# hstego.py embed <payload> <cover> <stego> <password>
ENTRYPOINT ["hstego.py"]
CMD ["--help"]

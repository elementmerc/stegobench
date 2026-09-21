# stegobench toolkit

One Docker image carrying seven steganography tools you can run without
installing anything else: `steghide`, `outguess`, `openstego`, `stegosuite`,
`zsteg`, `stegexpose` and `hstego`.

**Status: authored, not built.** This image has not been built or run. Build
it and run its self-tests before you trust it for anything. See "What has and
hasn't been checked" below.

## What's in it

| Tool | What it does | Kind |
|---|---|---|
| `steghide` | hides data in JPEG, BMP, WAV or AU files (graph-theoretic LSB) | embed / extract |
| `outguess` | hides data in JPEG files with statistical correction | embed / extract |
| `openstego` | hides data in PNG files (LSB, with or without a password) | embed / extract |
| `stegosuite` | hides AES-encrypted data spread across an image | embed |
| `zsteg` | looks for hidden data in PNG and BMP bit planes | detect |
| `stegexpose` | an ensemble of classical LSB detectors (Sample Pairs, RS, chi-square, Primary Sets) | detect |
| `hstego` | HILL and J-UNIWARD embedding with a real syndrome-trellis coder, the adaptive scheme the research literature assumes | embed / extract |

These are the seven tools measured at 118 MB to 1.41 GB each. They're bundled
together because a person doing everyday steganography work (a CTF, a quick
check on a file) reaches for tools in this size range, not for a nine
gigabyte deep learning stack.

## Size

Not yet measured, because the image hasn't been built. The seven source
images sum to roughly 4.2 GB, but that number overstates the built image by a
wide margin: five of the seven already shared a Debian or Java base before
this image existed, and this image goes further by putting all seven on one
Debian base with one set of shared libraries. Treat 4.2 GB as a loose upper
bound, not an estimate, until a real build reports the real number.

## Pulling it

```sh
docker pull ghcr.io/the-malware-files/stegobench/toolkit:latest
```

(This reference names where the image will live once published; it hasn't
been pushed yet. `IMAGES.md` in the parent directory records the digest of
every image that HAS been built and published.)

## Running each tool

The entrypoint dispatches on the first argument, so every tool is reached the
same way:

```sh
# steghide: hide secret.txt inside cover.jpg
docker run --rm -v "$PWD:/data" stegobench/toolkit \
  steghide embed -cf cover.jpg -ef secret.txt -sf stego.jpg -p PASS

# outguess: extract from a JPEG
docker run --rm -v "$PWD:/data" stegobench/toolkit \
  outguess -k PASS -r stego.jpg out.txt

# openstego: null-LSB embed into a PNG, no password
docker run --rm -v "$PWD:/data" stegobench/toolkit \
  openstego embed -mf secret.txt -cf cover.png -sf stego.png

# stegosuite: AES-encrypted embed
docker run --rm -v "$PWD:/data" stegobench/toolkit \
  stegosuite embed -f secret.txt -k PASS cover.png

# zsteg: scan a PNG for hidden bit-plane data
docker run --rm -v "$PWD:/data" stegobench/toolkit \
  zsteg -a stego.png

# stegexpose: scan a whole directory, prints a CSV verdict per file
docker run --rm -v "$PWD:/data" stegobench/toolkit \
  stegexpose /data

# hstego: embed with HILL at a given payload rate
docker run --rm -v "$PWD:/data" stegobench/toolkit \
  hstego embed cover.jpg secret.txt stego.jpg PASS
```

Running the image with no arguments prints a one-line banner naming the build
date, the commit it was built from, and the tools installed, so you can tell
at a glance whether the image you pulled is stale.

## Upgrading to the complete image

This image does not include Aletheia, the steganalysis parity reference we
benchmark against. Aletheia ships as two separate images instead:

| Image | Adds | Size |
|---|---|---|
| `aletheia` | classical detectors: SPA, RS, WS | 8.34 GB |
| `aletheia-rich` | rich-model detectors: SRM, DCTR, GFR (needs Octave) | 9.09 GB |

**Why Aletheia is separate rather than folded in.** Aletheia and its
rich-model variant carry a full scientific Python stack, and the rich-model
image also carries Octave. Together they are about 80% of the total bytes
across every tool this project has measured. Someone who wants to run
`steghide` on a CTF challenge should not be made to download a deep learning
stack first; someone who wants Aletheia's parity numbers knows what they are
asking for and can pull the larger image on purpose.

To get everything, pull both alongside the toolkit image:

```sh
docker pull ghcr.io/the-malware-files/stegobench/toolkit:latest
docker pull ghcr.io/the-malware-files/stegobench/aletheia:latest
docker pull ghcr.io/the-malware-files/stegobench/aletheia-rich:latest
```

Each image runs independently; there is no combined "complete" tag, because
building one would defeat the point of keeping the toolkit small for the
audience who does not need Aletheia.

## What has and hasn't been checked

- **Not built.** Building Docker images is heavy work that was deliberately
  kept off the machine this Dockerfile was written on. Nobody has run
  `docker build` against this file yet.
- **Not smoke-tested.** Each per-tool image this one is composed from has its
  own working Dockerfile and (where registered) a self-test fixture under
  `plugins/registry/`; this combined image inherits none of that automatically
  and needs its own round of embed/extract smoke tests once it builds.
- **SBOM not generated**, because an SBOM describes a built image and none
  exists yet. Once this image builds, generate one the same way any other
  published image here should (for example `syft ghcr.io/.../toolkit:latest
  -o spdx-json > toolkit.spdx.json`) and publish it alongside the image, per
  the constellation plan's Docker chapter.
- **Base image digests** are copied from the per-tool Dockerfiles in this
  repository (`tools/steghide/Dockerfile` and neighbours), which do build and
  run today; they were not re-verified against a live registry pull as part
  of authoring this file.

## Licences

Each tool keeps its own licence, as recorded in `plugins/registry/`:

| Tool | Licence |
|---|---|
| steghide | GPL-2.0 |
| outguess | BSD-3-Clause |
| openstego | GPL-2.0 |
| stegosuite | GPL-3.0 |
| zsteg | MIT |
| stegexpose | GPL-3.0 |
| hstego | MIT |

This image does not merge or relicense any of them; it packages them side by
side. Check the upstream project of whichever tool you redistribute results
from.

# stegobench toolkit

One Docker image carrying seven steganography tools you can run without
installing anything else: `steghide`, `outguess`, `openstego`, `stegosuite`,
`zsteg`, `hstego` and `stegcore`.

**Status: built and smoke-tested, not published.** A six-tool build of this
file (everything below except `stegcore`) was built on 2026-10-01 and measured
at 1.59 GB. Three tools were exercised end to end inside it. `stegcore` comes
from a container that is still private, so the full seven-tool image hasn't
been built yet. See "What has and hasn't been checked".

## What's in it

| Tool | What it does | Kind |
|---|---|---|
| `steghide` | hides data in JPEG, BMP, WAV or AU files (graph-theoretic LSB) | embed / extract |
| `outguess` | hides data in JPEG files with statistical correction | embed / extract |
| `openstego` | hides data in PNG files (LSB, with or without a password) | embed / extract |
| `stegosuite` | hides AES-encrypted data spread across an image | embed |
| `zsteg` | looks for hidden data in PNG and BMP bit planes | detect |
| `hstego` | HILL and J-UNIWARD embedding with a real syndrome-trellis coder, the adaptive scheme the research literature assumes | embed / extract |
| `stegcore` | an analysis ensemble: SPA, RS and WS with calibrated thresholds | detect |

They're bundled together because a person doing everyday steganography work (a
CTF, a quick check on a file) reaches for tools in this size range, not for a
nine gigabyte deep learning stack.

`stegcore` is a single 8.6 MB binary copied out of its own published image, so
it costs almost nothing here. It's in this registry as a **subject**, not a
yardstick (`plugins/registry/detectors/stegcore.toml` says why), and its
presence in this image doesn't change that.

### Why StegExpose isn't here any more

It was, until 2026-10-01. Two reasons it went:

- **No licence grant.** The upstream repository carries no LICENSE file and
  no licence header. Nothing gives anyone the right to redistribute it, so
  shipping it inside an image we publish wasn't ours to do. The registry entry
  records this as `licence = "none-granted"`.
- **The project is archived.** Upstream archived it publicly, so there's no
  one to ask and no fix coming.

The registry entry stays, so a user who has their own copy can still run it and
still gets a result that names what produced the number. It just isn't bundled.

## Size

**1.59 GB**, measured on 2026-10-01 against a build of this file without
`stegcore`. Adding the `stegcore` binary should move that by single-digit
megabytes. The seven source images sum to roughly 4.2 GB; the saving comes from
putting everything on one Debian base with one set of shared libraries and one
Java runtime instead of several.

## Pulling it

```sh
docker pull ghcr.io/the-malware-files/stegobench/toolkit:latest
```

(This reference names where the image will live once published; it hasn't been
pushed yet. `IMAGES.md` in the parent directory records the digest of every
image that HAS been built and published.)

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

# hstego: embed with HILL at a given payload rate
docker run --rm -v "$PWD:/data" stegobench/toolkit \
  hstego embed cover.jpg secret.txt stego.jpg PASS

# stegcore: analyse a file, JSON on stdout
docker run --rm -v "$PWD:/data" stegobench/toolkit \
  stegcore analyse --json stego.png
```

Running the image with no arguments prints a one-line banner naming the build
date, the commit it was built from, and the tools installed, so you can tell at
a glance whether the image you pulled is stale.

**Pass `--network=none`** unless a tool genuinely needs the network. None of
these do. The smoke tests below were all run that way.

## Upgrading to the complete image

This image does not include Aletheia, the steganalysis parity reference we
benchmark against. Aletheia ships as two separate images instead:

| Image | Adds | Size |
|---|---|---|
| `aletheia` | classical detectors: SPA, RS, WS | 8.34 GB |
| `aletheia-rich` | rich-model detectors: SRM, DCTR, GFR (needs Octave) | 9.09 GB |

**Why Aletheia is separate rather than folded in.** Aletheia and its rich-model
variant carry a full scientific Python stack, and the rich-model image also
carries Octave. Together they are about 80% of the total bytes across every
tool this project has measured. Someone who wants to run `steghide` on a CTF
challenge should not be made to download a deep learning stack first; someone
who wants Aletheia's parity numbers knows what they are asking for and can pull
the larger image on purpose.

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

**Checked**, on a six-tool build (this file minus `stegcore`) on 2026-10-01,
every run with `--network=none`:

| Check | Result |
|---|---|
| Image builds | yes, 1.59 GB |
| `openstego` embed then extract, PNG | recovered file byte-identical to the original; stego differs from the cover |
| `steghide` embed then extract, JPEG | recovered file byte-identical to the original |
| `zsteg -a` on a PNG | runs and reports |

The openstego round trip was the one that mattered: OpenStego 0.8.6 predates
Java 21 by some years, and Debian trixie has no `openjdk-17`, so the image runs
it on 21. That it works is now a measurement rather than an assumption.

**Not checked:**

- **The seven-tool image has never been built.** `stegcore` is copied from
  `ghcr.io/the-malware-files/stegcore`, which is still a private package, so a
  machine without a token can't pull it. The build works once that package is
  public.
- **`outguess`, `stegosuite` and `hstego` have not been run** in this image.
  They install and the binaries are on the PATH; nobody has put a file through
  them here.
- **SBOM not generated.** Once the full image builds, generate one the same way
  any other published image here should (for example
  `syft ghcr.io/.../toolkit:latest -o spdx-json > toolkit.spdx.json`) and
  publish it alongside the image.
- **Three `ARG *_REF` defaults still track `master`**, so two builds a week
  apart can differ. Pin them before any published benchmark.

## Licences

Each tool keeps its own licence, as recorded in `plugins/registry/`:

| Tool | Licence |
|---|---|
| steghide | GPL-2.0 |
| outguess | BSD-3-Clause |
| openstego | GPL-2.0 |
| stegosuite | GPL-3.0 |
| zsteg | MIT |
| hstego | MIT |
| stegcore | AGPL-3.0-or-later |

This image does not merge or relicense any of them; it packages them side by
side. Check the upstream project of whichever tool you redistribute results
from.

**Publishing this image carries a source-offer obligation.** `steghide`,
`openstego` and `stegosuite` are GPL, and `stegcore` is AGPL. An image we push
to a public registry is a distribution, which means the corresponding source
has to be offered alongside it. That offer doesn't exist yet and it's a
blocker on publishing, not a detail to sort out afterwards.

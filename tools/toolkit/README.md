# stegobench toolkit

One Docker image carrying seven steganography tools you can run without
installing anything else.

**Status: built and smoke-tested, not published.** 1.61 GB, built 2026-10-01.
See "What has and hasn't been checked".

| Tool | What it does | Kind |
|---|---|---|
| `steghide` | hides data in JPEG, BMP, WAV or AU files (graph-theoretic LSB) | embed / extract |
| `outguess` | hides data in JPEG files with statistical correction | embed / extract |
| `openstego` | hides data in PNG files (LSB, with or without a password) | embed / extract |
| `stegosuite` | hides AES-encrypted data spread across an image | embed |
| `zsteg` | looks for hidden data in PNG and BMP bit planes | detect |
| `hstego` | HILL and J-UNIWARD embedding with a real syndrome-trellis coder | embed / extract |
| `stegcore` | an analysis ensemble: SPA, RS and WS with calibrated thresholds | detect |

They're bundled together because a person doing everyday steganography work (a
CTF, a quick check on a file) reaches for tools in this size range, not for a
nine gigabyte deep learning stack.

`stegcore` is a single 8.6 MB binary copied out of its own published image, so
it costs almost nothing here. It's in this registry as a **subject**, not a
yardstick (`plugins/registry/detectors/stegcore.toml` says why), and being in
this image doesn't change that.

## Running it

Install:

```sh
docker pull ghcr.io/the-malware-files/stegobench/toolkit:latest
```

Run:

```sh
toolkit zsteg -a stego.png
```

...once you've made `toolkit` mean the long docker line, which you only do
once:

```sh
alias toolkit='docker run --rm --network=none -v "$PWD:/data" ghcr.io/the-malware-files/stegobench/toolkit'
```

Everything after `toolkit` is the tool's own name and its own arguments, and
your current directory is what it sees. `toolkit steghide ...`,
`toolkit stegcore ...`, and so on for all seven. Run `toolkit` by itself and
it lists them, with the build date and commit so you can tell whether what you
pulled is stale.

`--network=none` is in the alias on purpose: none of these tools needs the
network, and one that unexpectedly wants it should fail rather than reach.

## Aletheia is separate

Aletheia, the steganalysis parity reference, ships as two images of its own:
`aletheia` (classical detectors: SPA, RS, WS) at 8.34 GB and `aletheia-rich`
(SRM, DCTR, GFR, needs Octave) at 9.09 GB. Together they're about 80% of the
total bytes across every tool this project has measured, so someone who wants
to run `steghide` on a CTF challenge isn't made to download a deep learning
stack first. Pull them alongside the toolkit when you want them.

## Why StegExpose isn't here

It was, until 2026-10-01.

- **No licence grant.** The upstream repository carries no LICENSE file and no
  licence header, so nothing gives anyone the right to redistribute it.
- **The project is archived.** There's no one to ask and no fix coming.

The registry entry stays, so a user with their own copy still gets a result
naming what produced it. It just isn't bundled.

## What has and hasn't been checked

**Checked**, every run with `--network=none`:

| Check | Result |
|---|---|
| Seven-tool image builds | yes, 1.61 GB, 2026-10-01 |
| `stegcore analyse --json` | runs, version 4.1.0, JSON on a stego and a clean PNG |
| `openstego` embed then extract, PNG | payload recovered byte-identical; stego differs from the cover |
| `steghide` embed then extract, JPEG | payload recovered byte-identical |
| `zsteg -a` on a PNG | runs and reports |

The openstego round trip was the one that mattered: OpenStego 0.8.6 predates
Java 21 by years, and Debian trixie has no `openjdk-17`, so the image runs it
on 21. That it works is a measurement rather than an assumption.

**Not checked:**

- **`outguess`, `stegosuite` and `hstego` have not been run here.** They
  install and are on the PATH; nobody has put a file through them.
- **SBOM not generated.** Do it once the image is ready to publish
  (`syft ghcr.io/.../toolkit:latest -o spdx-json > toolkit.spdx.json`).
- **Three `ARG *_REF` defaults still track `master`**, so two builds a week
  apart can differ. Pin them before any published benchmark.

## Licences, and the source you're owed

| Tool | Licence |
|---|---|
| steghide | GPL-2.0 |
| outguess | BSD-3-Clause |
| openstego | GPL-2.0 |
| stegosuite | GPL-3.0 |
| zsteg | MIT |
| hstego | MIT |
| stegcore | AGPL-3.0-or-later |

This image doesn't merge or relicense any of them; it packages them side by
side.

Publishing it carries a source-offer obligation, and the Debian base is the
large part of that: a few hundred packages, most of them copyleft.
`SOURCE-OFFER.md` covers what's owed and `collect_sources.py` builds the
archive (569 files, about a gigabyte, read out of the image itself so the
versions can't drift from the binaries).

# The toolchain, and which images built the corpus

Every tool Stegobench drives runs in a container, so that a result depends on a
recorded thing rather than on whatever happened to be installed.

## What built Pentimento Core v1.0.0

These are the exact images. The corpus was built by these and by nothing else,
so an attempt to reproduce it that uses a different build of `steghide` is
reproducing something adjacent.

| Tool | Image ID | Built | Size |
|---|---|---|---|
| aletheia | `sha256:3f19136f8f3e1479fc60e9986d8d2431818ca6dc4471ae1e49c5076acfccf87d` | 2026-09-17 | 8.34 GB |
| aletheia-rich | `sha256:5b08e93aaed2b2c30753ec3654df41d1406a4669f998384a04c26e24952f3ddf` | 2026-09-17 | 9.09 GB |
| hstego | `sha256:59710f7b5fbaeb7c3b1d4333e64654c1721a3ddb60b489d8e54d5d0e8b269bfb` | 2026-09-16 | 1.41 GB |
| openstego | `sha256:c8be427da41a1f0389762ee9c50ac27b7dc7ff14013d2dc83b361925da722ee2` | 2026-09-16 | 911 MB |
| outguess | `sha256:53eed9db453ffddd2ae2c0ec9877431def12d7340864c047e60dc546ed64aa2f` | 2026-09-16 | 118 MB |
| stegexpose | `sha256:70c545a890a0e0b1695998525b6d4fa270ece2619108e9a80286f237593980b5` | 2026-09-17 | 512 MB |
| steghide | `sha256:69ecb87c0f9cf99ff48636b1de7d7f8ea56d84d508cf65e5486e0acbe3c97150` | 2026-09-16 | 120 MB |
| stegosuite | `sha256:f3d9110af92cf4eea396a9a8b4aaa61bc67160e2f316b1db36f57f9b9f216906` | 2026-09-16 | 836 MB |
| zsteg | `sha256:00c110ec4a37b87449872a565c2bb86eb6e93f28fde27829d98d1c7168ba68db` | 2026-09-17 | 287 MB |

Only `steghide` and `outguess` produced corpus images. The rest are detectors,
and they affect measurements rather than data.

## Why the Dockerfile is not enough

Each Dockerfile here now pins its base by digest, which stops the base moving
under us. It does not make a rebuild byte-identical, because:

- `apt-get install steghide` takes whatever version the Debian archive serves
  that day. Pinning the exact version is possible; pinning what that version
  was *built against* is not, from here.
- `steghide` and `outguess` are C programs whose output depends on the libjpeg
  they were linked against. A different libjpeg gives different bytes for the
  same input and the same payload.

So the Dockerfile is the recipe and the image is the artefact. For a corpus to
be checkable, the artefact is what has to be reachable, which is why the images
are published rather than only described.

## Where the definitions used to live

Six of these were in this repository and four were in a different project's
private directory, with `aletheia` and `stegexpose` duplicated between the two
in identical copies. `steghide` and `outguess`, which build corpus images, were
in the private half only, so this repository could not build its own
dependencies and the reproducibility claim could not have been met by anyone
who cloned it.

They are all here now.

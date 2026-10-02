# Running the harness in a container

If you don't want a Rust toolchain on your machine, you can build `stegobench`
into a container image and run it from there. The repository carries the
`Dockerfile` that does it.

This page is about the **harness** image: `stegobench` itself. It isn't about
the detector and embedder images the registry names. Those are other people's
tools, built elsewhere, and the section at the bottom says what the difference
means for you in practice.

## Build it

```sh
git clone https://github.com/elementmerc/stegobench
cd stegobench
docker build -t stegobench .
```

The build has two stages. The first compiles the binary with the Rust version
`rust-toolchain.toml` pins, and the second copies just the binary into a small
Alpine layer, so the image you run carries no compiler. Both base images are
named by digest rather than by tag, so the same `docker build` resolves the
same bases tomorrow.

The result is about 33 MB.

## Run it

```sh
docker run --rm stegobench --version
```

```
stegobench 1.0.0
https://github.com/elementmerc/stegobench
```

The binary is the entry point, so every command reads the way it does on a
host. There's nothing to configure and nothing to mount for the tool to know
what it is:

```sh
docker run --rm stegobench list detectors
```

```
aletheia-rich MIT               container stegobench/aletheia-rich
aletheia-rs   MIT               container stegobench/aletheia
aletheia-spa  MIT               container stegobench/aletheia
stegashield   proprietary       container 5iprojects/stegashield  needs STEGASHIELD_LICENCE
stegcore      AGPL-3.0-or-later local     stegcore
stegexpose    none-granted      container stegobench/stegexpose
zsteg         MIT               container stegobench/zsteg

...

registry  built in
```

`registry  built in` is the line that matters. The tool registry, the self
test fixtures, the host adapters and the starter corpus are all compiled into
the binary, so none of them has to be mounted and none of them can be a
version older than the binary reading it.

## What `doctor` says, and why it's right

```sh
docker run --rm stegobench doctor
```

```
registry  built in
fixtures  built in

DETECTORS (7)
aletheia-rich   unknown   no container runtime: docker is not on PATH  not verified
    No container runtime answered. Install Docker (or a drop-in replacement) and start the daemon.
    docker pull stegobench/aletheia-rich@sha256:5b08e93aaed2b2c30753ec3654df41d1406a4669f998384a04c26e24952f3ddf
        about 9090 MB, pinned by digest
...
13 tool(s): 7 detector(s), 6 embedder(s).
On this machine: 0 installed, 3 not installed, 10 undetermined, 0 cannot run here.
UNFIT: no tool here is usable, so nothing could be measured.
```

It exits non zero and says the machine is unfit. **That's the correct answer,
not a broken image.** Most of the detectors are themselves containers, and
there's no container runtime inside this container to start them with, so
`doctor` reports what it genuinely could not determine rather than guessing.
Each line names the thing it's missing.

There's no docker client in the image on purpose. Adding one would only help
somebody who then mounted the host's docker socket into the container, and
that mount is root on the host: anything that can talk to the socket can start
a privileged container with your filesystem inside it. We won't ship an image
whose documented usage is a way to become root on your own machine.

So the honest summary: **use the image for the parts of the tool that don't
need a container runtime**, and install the harness on the host when you want
to score with a containerised detector. The parts that work in the image are
`schema`, `validate`, `verify`, `list`, `describe`, `metrics`, `report`,
`plan`, `fetch`, `completions` and `help`, which is most of what you do around
a run rather than during one.

## Mounting your own files

The container works in `/work`, so mount what you want it to read there:

```sh
docker run --rm -v "$PWD:/work" stegobench validate result.json
```

It runs as an unprivileged user with the fixed id 65532, which means it can
read your files only if they're readable by others, and can't write into your
directory at all. If you want it to write (a result document, a report, a
completion script), tell it to run as you:

```sh
docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/work" stegobench report result.json --out table.md
```

Anything it writes then belongs to you, with your ownership and your
permissions. Without that flag the output would be owned by a user that
doesn't exist on your machine, which is a mess to clean up afterwards.

## Two kinds of container, and why only one of them is here

The registry pins thirteen tools, and most of them are containers:

```
docker pull stegobench/zsteg@sha256:00c110ec4a37b87449872a565c2bb86eb6e93f28fde27829d98d1c7168ba68db
```

Those images are not published anywhere yet. They were built on the machines
that run them, and the digests above name bytes that exist on those machines
and nowhere else. If you try that `docker pull`, it will fail, because there's
nothing on the other end to pull from.

That's a real limit on what you can reproduce today, and it's stated here
rather than left for you to discover: a number this project publishes from a
containerised detector is one you can't currently measure again yourself,
because you can't get the detector. What you can check today is everything that
doesn't need those images: the schemas, the result documents, the metrics
arithmetic, the corpus digests, and any detector you install yourself.

## Other things the image doesn't carry

- **Python.** One registry entry, `stegashield`, is a service reached by a
  Python adapter running next to the tool rather than inside it. `doctor` in
  the image reports it as missing python3, which is accurate. Installing a
  Python runtime to support a single proprietary entry that also needs you to
  start a server of your own isn't worth tripling the size of the image.
- **Any detector.** Nothing registered is bundled. The image is the harness.

## What's in the image

| Path | What it is |
|---|---|
| `/usr/local/bin/stegobench` | The binary, statically linked, no shared libraries |
| `/usr/share/man/man1/` | 15 man pages, generated from the same command tree the binary parses against |
| `/usr/share/doc/stegobench/` | `LICENSE`, `README.md` and `CHANGELOG.md` |
| `/usr/bin/curl` | What `stegobench fetch` shells out to |
| `/work` | The working directory, where you mount your own files |

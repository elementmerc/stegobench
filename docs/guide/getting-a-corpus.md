# Getting a corpus

You have just installed the tool and you have nothing to score. This is how you
get images whose answers are already known.

## The thirty-second version

A corpus ships in the box, so the first run needs no download at all:

```sh
stegobench score --corpus corpora/starter --detector <name>
```

Six covers and twelve stego images. It proves the machinery works and it is not
a measurement: an AUC over eighteen images moves by about a tenth when one
image changes. Do not quote a number from it. `corpora/starter/README.md` says
the same thing at more length.

## What is registered, and what you may do with it

```sh
stegobench list corpora
stegobench describe pentimento-core
```

`list` prints one line per corpus with two facts that are easy to confuse:

| Column | Question it answers |
|---|---|
| The licence | May you use it |
| `republish` | May you pass it on |

They are separate fields because they are separate answers. ALASKA2 and
BOSSbase may both be used and neither may be republished, for two different
reasons, and `describe` prints what was read and when.

## Fetching one

**One corpus is fetchable: the starter.** It needs no download route and no
network, because it is compiled into the binary:

```sh
stegobench fetch stegobench-starter --tier nano --out ./starter
```

Six covers and twelve stego images. It's a demonstration, and a number from it
must not be quoted.

**Nothing else is fetchable yet.** No other registered corpus declares a
download route, so `stegobench fetch` refuses whichever id you give it: exit
code 7 for ALASKA2 and BOSSbase, whose terms say no, and exit code 3 for the
rest, which have nothing on file to fetch. The message names the reason and
points at `describe`. The rest of this section describes what the command does
once a route is on file. Until then, obtain a registered corpus by the route
`describe` prints, or build your own.

```sh
stegobench fetch <corpus id> --tier <tier>
```

Three things happen before a single byte is downloaded.

**The terms are read.** A corpus whose terms do not permit redistribution is
refused, and nothing opens. Fetching somebody else's dataset on your behalf
would make this project a mirror of it, and `docs/design/cover-source-licensing.md`
is a catalogue of what happens when a mirror decides otherwise. You can still
use those corpora: obtain them yourself by the route `describe` prints, then
point `score` at the copy you obtained.

**The size is checked.** The registry declares how many bytes the route serves,
so a tier larger than you meant to fetch is refused rather than discovered an
hour in. `--max-bytes` lowers the ceiling.

**A digest is already on file.** The registry declares the SHA-256 of the bytes
before the download starts, so what arrives can be checked against something
that did not come from the same place. Bytes that do not match are thrown away,
not kept.

What you get back is one verified file and its path. The tool does not unpack
it; it tells you what it is and prints the line that does.

### If it stops halfway

Press Ctrl+C, close the laptop, lose the connection: the partly downloaded
bytes are kept and the next run of the same command continues from them. There
is never a half-written file at the name that means "verified", so a download
that was interrupted can never be scored as though it were whole. Running the
command again when the file is already here costs nothing and touches no
network.

### What it needs

`curl`, on your PATH. It carries the bytes; everything else (the resume, the
size ceiling, the digest check) happens here. curl ships with macOS and with
Windows 10 and later, and is `sudo apt install curl` on Debian and Ubuntu. If
it is missing, the command says so and downloads nothing.

### Where it puts things

Under your user data directory by default, laid out by content address, so the
same tier fetched from two different folders is downloaded once. `--dest` names
somewhere else, and `STEGOBENCH_CORPUS_DIR` sets it for good.

## Building your own instead

Everything under `generators/` builds a corpus from photographs you fetch
yourself, with a licence recorded per file. [Build a corpus](/guide/build-a-corpus)
walks it. The one rule that matters more than any other is that a cover and its
stego twin must come out of the same source array through the same code path,
so they differ in the payload and nothing else: [Pairing](/guide/pairing) says
why three measurements have already been lost to breaking it.

## Before you quote a number

[Limitations](/guide/limits).

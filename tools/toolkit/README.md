# stegobench toolkit

Seven ready-made tools for hiding files inside pictures, and for trying to spot
when somebody else has. One Docker image, nothing else to install.

**It is a box of other people's tools.** It does not do anything itself.

## Getting it

Not published yet, so build it:

```sh
tools/toolkit/build.sh stegobench/toolkit
```

About twenty minutes and 1.61 GB. Once it's published this becomes a
`docker pull` and this section will say so.

## Running it

Set this up once:

```sh
alias toolkit='docker run --rm -it --network=none --user "$(id -u):$(id -g)" -v "$PWD:/data" stegobench/toolkit'
```

Then:

```sh
toolkit                       # what is in the box
toolkit stegcore wizard       # guided, if you have not done this before
toolkit steghide --help       # any tool's own help
```

Everything after `toolkit` is a tool's name and that tool's own arguments, and
your current directory is what it sees.

**Each part of that alias earns its place.** `--network=none` because none of
these tools needs the network and one that suddenly wants it should fail rather
than reach. `--user` because without it everything the tools write is owned by
root and you cannot delete your own output. `-it` because the guided wizard
needs a keyboard, and without it that wizard exits reporting that you cancelled
when you did not.

## A worked example, in two commands

```sh
toolkit stegcore embed photo.png message.txt -o secret.png --passphrase hunter2
toolkit stegcore extract secret.png --passphrase hunter2
```

The second writes your file back out. Compare it with the original and it is
byte for byte the same. That is the whole idea.

## What's in it

| Tool | What it does |
|---|---|
| `stegcore` | hides and finds; has a guided wizard, and the friendliest output of the seven |
| `steghide` | hides a file in a JPEG, BMP or WAV, with a passphrase |
| `outguess` | hides a file in a JPEG, and adjusts the image afterwards to look more ordinary |
| `openstego` | hides a file in a PNG |
| `stegosuite` | hides a file in an image, encrypted, spread across it |
| `hstego` | hides a file by putting it where the picture is busiest, which is harder to spot. This is what current research uses |
| `zsteg` | looks for hidden data in PNG and BMP files |

They're bundled because somebody doing everyday work (a puzzle, a quick check
on a file) reaches for tools this size, not for a nine gigabyte research stack.

## Reading what the detectors tell you

**This is the part that misleads people, so read it before you trust an
answer.**

`zsteg` prints a few hundred lines listing everything it tried. Most of those
lines are noise. It will cheerfully report things like
`file: OpenPGP Public Key Version 6` or `SVR2 executable` **about a completely
ordinary photograph**, because it is pattern-matching against random-looking
bits and in a big enough pile of random bits something always matches. A hit
from `zsteg` is a reason to look closer, never an answer on its own.

`stegcore analyse` draws a tidy box with five scores in it and a one-word
verdict. The scores are five different statistical tests, and they disagree
with each other often. A word like "Suspicious" on a photo you have no reason
to doubt usually means the photo is noisy, not that something is hidden.

**Treat "Clean" as meaning nothing.** This was measured, not assumed. Hide a
file with `steghide`, then ask `stegcore analyse` about it a minute later, and
it reports `✓ Clean`. A green tick means "I found nothing I know how to look
for", not "there is nothing here".

**What it is actually good at is naming the tool.** When it does find
something it prints a line like `Signature: OpenStego (exact signature)`, and
that is the single most useful thing any detector here prints, because it tells
you which tool to use to get the payload out. In a side-by-side on a clean file
and a stego file, the five statistical scores were near identical
(48/21/4/100/24 against 48/21/6/100/27) and the verdict came entirely from
that signature line. Read the signature; treat the bars as decoration.

**No detector here will reliably tell you whether a specific file has something
in it.** A good hiding tool is designed to defeat exactly these tests, and
mostly succeeds. That is not a fault in either half; it is what the field is
like. If you need numbers you can defend rather than a verdict, that is what
the `stegobench` tool in this repository is for.

## Things that look like faults and are not

- **`openstego` prints nothing at all on a successful embed.** Check with
  `ls`; silence means it worked.
- **`steghide` overwrites its own progress line**, so a successful embed can
  read as a doubled, mangled sentence. It is cosmetic.
- **`steghide` asks before overwriting an existing output file.** Without
  `-it` it cannot ask, and fails with "could not get terminal attributes".
- **`stegcore wizard` needs a real terminal.** Over ssh without `-it` it can
  print nothing at all and appear to hang.

## Aletheia is separate

Aletheia is a research-grade detection suite we compare against. It ships as
two images of its own, 8.34 GB and 9.09 GB, because it carries a full
scientific Python stack. Together they are about 80% of the bytes across every
tool this project has measured, so somebody who wants to run one small tool on
one file is not made to download all that first. Pull them when you need them.

## What has and hasn't been checked

**Checked**, every run with `--network=none`:

| Check | Result |
|---|---|
| Seven-tool image builds | yes, 1.61 GB, 2026-10-01 |
| `stegcore` embed then extract | payload recovered byte-identical |
| `openstego` embed then extract, PNG | payload recovered byte-identical; stego differs from the cover |
| `steghide` embed then extract, JPEG | payload recovered byte-identical |
| `zsteg -a` on a PNG | runs and reports |

The `openstego` round trip was the one that mattered: it predates Java 21 by
years, and Debian trixie has no `openjdk-17`, so the image runs it on 21. That
it works is a measurement rather than an assumption.

**Not checked:** `outguess`, `stegosuite` and `hstego` have not had a file put
through them here. They install and are on the PATH.

**Also outstanding:** three `ARG *_REF` build arguments still track `master`, so
two builds a week apart can differ; pin them before any published benchmark.
No SBOM yet.

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
side. Publishing it carries a source-offer obligation, and the Debian base is
the large part of that. `SOURCE-OFFER.md` covers what's owed and
`collect_sources.py` builds the archive.

**StegExpose used to be here and was removed on 2026-10-01**, because its
upstream grants no licence at all: no LICENSE file, no header, and the project
is archived, so there is nobody to ask. Its registry entry stays, so somebody
with their own copy still gets a result that names what produced it.

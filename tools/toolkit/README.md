# stegobench toolkit

Seven ready-made tools for hiding files inside pictures, and for trying to spot
when somebody else has. One Docker image, nothing else to install.

It is a box of other people's tools. It does not do anything itself.

## Getting it

Not published yet, so build it. About twenty minutes and 1.61 GB.

```sh
tools/toolkit/build.sh stegobench/toolkit:latest
```

## Running it

Set this up once:

```sh
alias toolkit='docker run --rm $([ -t 0 ] && printf -- "-it") --network=none --read-only --tmpfs /tmp:rw,noexec,nosuid --security-opt no-new-privileges --cap-drop ALL --user "$(id -u):$(id -g)" -v "$PWD:/data" stegobench/toolkit:latest'
```

It is one line and one paste, and it is the locked-down version rather than
the minimal one: no network, read-only filesystem, no capabilities, no
privilege escalation, running as you. Every tool in the image was put through
an embed and extract round trip under exactly those flags, so nothing here
costs you a feature. `docs/guide/toolkit.md` says what each part does.

The tag here must be the tag you built. If you built it as something else,
change it in both places, or docker goes looking for a `:latest` that does not
exist and reports `pull access denied`, which reads like a login problem and
is not one.

Then:

```sh
toolkit                                 # what is in the box
toolkit stegcore analyse /data/photo.png    # does anything look hidden
toolkit steghide --help                 # any tool's own help
```

Everything after `toolkit` is a tool's name and that tool's own arguments, and
your current directory is what it sees. Every part of that alias earns its
place; `docs/guide/toolkit.md` says why.

## A worked example, in two commands

```sh
toolkit stegcore embed photo.png message.txt -o secret.png --passphrase hunter2
toolkit stegcore extract secret.png --passphrase hunter2
```

Your file comes back out byte for byte. That is the whole idea.

## What's in it

| Tool | What it does |
|---|---|
| `stegcore` | hides and finds; the friendliest output of the seven |
| `steghide` | hides a file in a JPEG, BMP or WAV, with a passphrase |
| `outguess` | hides a file in a JPEG, adjusting the image to look more ordinary |
| `openstego` | hides a file in a PNG |
| `stegosuite` | hides a file in an image, encrypted, spread across it |
| `hstego` | hides a file where the picture is busiest, which is harder to spot. What current research uses |
| `zsteg` | looks for hidden data in PNG and BMP files |

## Before you trust a detector

**Neither verdict means anything on its own.** Measured twice. A file hidden
with `steghide` comes back `✓ Clean` a minute later; an untouched original
photograph comes back `⚠ Suspicious`. On ordinary photos the verdict tracked
the file format rather than the contents.

**`zsteg` reports hidden PGP keys and executables inside ordinary
photographs.** That is noise, not a finding.

**`stegosuite extract` overwrites files in your working directory** and cannot
be told not to. Extract into an empty directory.

**`outguess` can hand back a file of the right size with a wrong byte in it,
and report success.** Check what you extracted against a hash of what you hid.

`docs/guide/toolkit.md` explains all of these, and what to use when you need a
number you can defend.

## Where this came from

| | |
|---|---|
| Source | https://github.com/elementmerc/stegobench |
| Licences, one per package | `/usr/share/doc/<package>/copyright` inside the image |
| What is in it, machine readable | `tools/toolkit/collect_sources.py <image> --out sbom --sbom-only` writes a CycloneDX bill of materials in about a minute |
| `stegcore`'s acceptable use policy | https://github.com/The-Malware-Files/Stegcore/blob/main/AUP.md |

`toolkit` with no arguments prints the first, second and fourth of those, so
you do not have to come back here for them.

## The rest

| | |
|---|---|
| Why the alias looks like that, reading detector output, things that look like faults and are not, what has and has not been tested | `docs/guide/toolkit.md` |
| Licences, and the source you are owed if we publish this | `SOURCE-OFFER.md` |
| A research-grade detection suite, 17 GB across two images | `tools/aletheia/` |

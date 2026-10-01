# stegobench toolkit

Seven ready-made tools for hiding files inside pictures, and for trying to spot
when somebody else has. One Docker image, nothing else to install.

It is a box of other people's tools. It does not do anything itself.

## Getting it

Not published yet, so build it. About twenty minutes and 1.61 GB.

```sh
tools/toolkit/build.sh stegobench/toolkit
```

## Running it

Set this up once:

```sh
alias toolkit='docker run --rm -it --network=none --user "$(id -u):$(id -g)" -v "$PWD:/data" stegobench/toolkit'
```

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

**A "Clean" verdict means nothing.** Measured: hide a file with `steghide`,
ask `stegcore analyse` a minute later, and it says `✓ Clean`. Read the
`Signature:` line instead, which names the tool that did the embedding.

**`zsteg` reports hidden PGP keys and executables inside ordinary
photographs.** That is noise, not a finding.

`docs/guide/toolkit.md` explains both, and what to use when you need a number
you can defend.

## The rest

| | |
|---|---|
| Why the alias looks like that, reading detector output, things that look like faults and are not, what has and has not been tested | `docs/guide/toolkit.md` |
| Licences, and the source you are owed if we publish this | `SOURCE-OFFER.md` |
| A research-grade detection suite, 17 GB across two images | `tools/aletheia/` |

# The toolkit image, in detail

`tools/toolkit/README.md` is the short version: how to get the image and how
to run it. This is everything a reader needs once they are actually using it.

## Why the alias looks like that

```sh
alias toolkit='docker run --rm -it --network=none --user "$(id -u):$(id -g)" -v "$PWD:/data" stegobench/toolkit'
```

Each part is there because leaving it out broke something a real person hit.

| Part | Why |
|---|---|
| `--network=none` | None of these tools needs the network. One that suddenly wants it should fail rather than reach out |
| `--user` | Without it everything the tools write is owned by root, mode 600, and you cannot delete your own output on a shared machine |
| `-it` | The guided wizard needs a keyboard. Without it, it exits reporting that you cancelled when you did not. `steghide` also prompts before overwriting and fails with "could not get terminal attributes" when nothing is listening |
| `-v "$PWD:/data"` | The tools see your current directory as `/data`. Paths you pass them are paths inside the container |

## Reading what the detectors tell you

This is the part that misleads people. Read it before you trust an answer.

### A "Clean" verdict means nothing

Not a caution: a measurement. Hide a file with `steghide`, then ask
`stegcore analyse` about it sixty seconds later, and it reports `✓ Clean`.

A green tick means "I found nothing I know how to look for", never "there is
nothing here".

### What `stegcore analyse` is actually good at

Naming the tool. When it finds something it prints a line like:

```
Signature: OpenStego (exact signature)
```

That is the most useful thing any detector here prints, because it tells you
which tool to use to get the payload out.

Side by side on a clean PNG and a stego file built from that same cover, the
five statistical scores were near identical:

| | Chi-Sq | SPA | RS | LSB Entropy | Weighted Stego |
|---|---|---|---|---|---|
| clean | 48% | 21% | 4% | 100% | 24% |
| stego | 48% | 21% | 6% | 100% | 27% |

The verdict came entirely from the signature line. Read the signature; treat
the bars as decoration.

### `zsteg` output is mostly noise

`zsteg -a` prints a few hundred lines listing everything it tried. It will
report things like `file: OpenPGP Public Key Version 6` or `SVR2 executable`
**about a completely ordinary photograph**, because it is pattern-matching
against random-looking bits, and in a big enough pile of random bits something
always matches.

A hit from `zsteg` is a reason to look closer, never an answer.

### Why no detector here will just tell you

A good hiding tool is designed to defeat exactly these tests, and mostly
succeeds. That is not a fault in either half; it is what the field is like.

If you need a number you can defend rather than a verdict, that is what the
`stegobench` tool in this repository is for: it measures detectors against
images whose answers are already known, and reports how often each was right.

## Things that look like faults and are not

- **`openstego` prints nothing at all on a successful embed.** Check with
  `ls`; silence means it worked.
- **`steghide` overwrites its own progress line**, so a successful embed can
  read as a doubled, mangled sentence.
- **`steghide` asks before overwriting an existing output file.** Without
  `-it` it cannot ask, and fails with "could not get terminal attributes".
- **`stegcore wizard` needs a real terminal.** Over ssh without `-it` it can
  print nothing at all and appear to hang.
- **The banner's commit field can read `not-a-git-checkout`.** That means the
  image was built outside a clone, which is honest rather than broken.

## What has and hasn't been checked

**Checked**, every run with `--network=none`:

| Check | Result |
|---|---|
| Seven-tool image builds | yes, 1.61 GB, 2026-10-01 |
| `stegcore` embed then extract | payload recovered byte-identical |
| `openstego` embed then extract, PNG | payload recovered byte-identical; stego differs from the cover |
| `steghide` embed then extract, JPEG | payload recovered byte-identical |
| `zsteg -a` on a PNG | runs and reports |
| All of the above under `--user` | clean, output owned by the caller |

The `openstego` round trip was the one that mattered: it predates Java 21 by
years, and Debian trixie has no `openjdk-17`, so the image runs it on 21. That
it works is a measurement rather than an assumption.

**Not checked.** `outguess`, `stegosuite` and `hstego` have not had a file put
through them here. They install and are on the PATH.

**Outstanding.** Three `ARG *_REF` build arguments still track `master`, so two
builds a week apart can differ; pin them before any published benchmark. No
SBOM yet.

## Why StegExpose is not in the image

It was, until 2026-10-01. Its upstream grants no licence at all: no LICENSE
file, no header, and the project is archived, so there is nobody to ask.

Its registry entry stays, so somebody with their own copy still gets a result
that names what produced it. It is simply not bundled.

## Aletheia is separate

Aletheia is a research-grade detection suite this project compares against. It
ships as two images of its own, 8.34 GB and 9.09 GB, because it carries a full
scientific Python stack. Together they are about 80% of the bytes across every
tool this project has measured, so somebody who wants to run one small tool on
one file is not made to download all that first.

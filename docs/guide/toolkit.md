# The toolkit image, in detail

`tools/toolkit/README.md` is the short version: how to get the image and how
to run it. This is everything a reader needs once they are actually using it.

## Why the alias looks like that

```sh
alias toolkit='docker run --rm $([ -t 0 ] && printf -- "-it") --network=none --user "$(id -u):$(id -g)" -v "$PWD:/data" stegobench/toolkit:latest'
```

Each part is there because leaving it out broke something a real person hit.

| Part | Why |
|---|---|
| `--network=none` | None of these tools needs the network. One that suddenly wants it should fail rather than reach out |
| `--user` | Without it everything the tools write is owned by root, mode 600, and you cannot delete your own output on a shared machine |
| `$([ -t 0 ] && printf -- "-it")` | Adds `-it` only when you are actually at a keyboard. Interactive tools need it: the guided wizard exits reporting that you cancelled when you did not, and `steghide` prompts before overwriting and fails with "could not get terminal attributes". But passing `-it` unconditionally breaks every scripted or piped run with `the input device is not a TTY`, so the alias asks rather than guessing |
| `-v "$PWD:/data"` | The tools see your current directory as `/data`. Paths you pass them are paths inside the container |
| the tag | Must match the tag you built. Omit it and docker looks for a `:latest` that may not exist, and reports `pull access denied`, which reads like a login problem and is not one |

## Reading what the detectors tell you

This is the part that misleads people. Read it before you trust an answer.

### Neither verdict means anything on its own

Measured, twice, by different people.

**"Clean" does not mean clean.** Hide a file with `steghide`, then ask
`stegcore analyse` about it sixty seconds later, and it reports `✓ Clean`.

**"Suspicious" does not mean suspicious.** An *unmodified original*
photograph came back `⚠ Suspicious` at 16%, identically to two stego files
made from it. Across six files, every PNG and BMP was flagged suspicious
whether or not anything was hidden, and every JPEG was cleared whether or not
anything was hidden.

So on an ordinary photograph the verdict tracked the file format and not the
contents. Do not put it in front of anybody as evidence.

### The `Signature:` line, which is inconsistent

When `stegcore analyse` does recognise an embedding tool it prints a line
like `Signature: OpenStego (exact signature)`, and that is genuinely useful
because it names the tool to use to get the payload out.

**It does not always appear, and we cannot tell you when it will.** One tester
saw it on an openstego file. A second tester could not reproduce it at all:
six files from three embedders, including stegcore's own output, every one
returning `"tool_fingerprint": null` in the machine-readable output.

Treat the line as a bonus when it shows up, never as something to rely on, and
never read its absence as evidence that a file is clean.

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

## Using hstego, which the research literature cares about

`hstego` puts the payload where the picture is busiest, which is what current
research benchmarks against rather than plain LSB. Four facts about it are not
obvious from its help, and a researcher had to recover every one of them by
experiment before they could quote a number.

**There is no payload rate flag.** The rate is set by how big your message
file is. Nothing else controls it.

**The formula:**

```
bits per pixel = (message_bytes + 71) * 8 / (width * height * channels)
```

The 71 bytes are flat tool overhead with no block padding, so a payload rate
you quote must say whether it counts your message or the embedded blob.

**Capacity is capped at about 0.05 bits per pixel per channel.** Measured at
0.0503 bpp by fitting capacity against pixel count across five image sizes.
Ask a specific cover with:

```sh
toolkit hstego capacity /data/photo.png
```

**This ceiling is the most important fact about the tool if you are comparing
with published work.** Papers using HILL, S-UNIWARD or J-UNIWARD typically run
0.05 to 0.4 bits per pixel. This implementation reaches only the very bottom
of that range and there is no flag to raise it, so most direct comparisons are
not possible.

**Embedding is not deterministic.** The same cover, message and password run
twice produce different files. Archive the stego images themselves; a seed and
a command line will not reproduce the set.

**The adaptivity is real**, and was measured rather than assumed. On a cover
built as a flat half and a noisy half, embedding changed **zero pixels in the
flat half and 2,224 in the noisy half**. Every change is plus or minus one,
at 7.5 to 7.9 bits per change, which is what a syndrome-trellis coder against
a distortion function looks like.

**What we cannot tell you.** The software names no scheme, so we cannot say
which distortion function it implements; `toolkit versions` reports the
package version to cite instead. We also cannot tell you whether the JPEG path
is side-informed, and that distinction matters for comparability.

## One thing that will bite you

**`stegosuite extract` overwrites files in your working directory and cannot
be told not to.**

It has no output option. It writes out the *embedded original filename*, into
wherever you are standing. If that name matches a file you already have, yours
is gone: no prompt, no warning, exit 0.

This is worse than it sounds, because the file it leaves behind looks correct.
If you extract into the directory holding the payload you embedded, you
destroy your original and the result still matches, so nothing tells you
anything happened.

Extract into an empty directory, always.

## Things that look like faults and are not

- **`openstego` prints nothing at all on a successful embed.** Check with
  `ls`; silence means it worked.
- **`steghide` overwrites its own progress line**, so a successful embed can
  read as a doubled, mangled sentence.
- **`steghide` asks before overwriting an existing output file.** Without
  `-it` it cannot ask, and fails with "could not get terminal attributes".
- **`stegcore wizard` needs a real terminal.** Over ssh without `-it` it can
  print nothing at all and appear to hang.
- **`hstego` is silent on a successful embed**, like openstego.
- **`hstego` prints warnings and debug lines on stdout**, including an ImageIO
  `DeprecationWarning` on PNG work and `coeffs shape:` / `precover shape:` /
  `coeffs_estim shape:` on JPEG work. Anything parsing stdout should expect
  them.
- **`hstego --help` prints its own internal container path**, so the example
  it shows cannot be copied and run as written. Use `toolkit hstego ...`.
- **`steghide info` exits 1 with "could not extract any data with that
  passphrase!"** when there is nothing hidden, after correctly printing the
  capacity. That is the answer, not an error.
- **`outguess` recompresses the JPEG it writes**, forcing quality 75. A
  256,900-byte cover came out at 138,942 bytes with visible quality loss. If
  your point is that the picture is unchanged, use a different tool.
- **`outguess` prints an integer underflow**: `Correctable message size:
  18446744073709539295 bits`. Cosmetic, upstream.

## Exit codes are not reliable across these tools

Two of the six report a failed extraction with exit status 0:

| Tool | Wrong passphrase gives |
|---|---|
| `stegcore` | message, exit 2 |
| `steghide` | message, exit 1 |
| `hstego` | `WARNING: message not found`, zero-byte output, **exit 0** |
| `openstego` | `Embedded data is corrupt OR invalid password...`, **exit 0** |

If you are scripting any of this, check the size of the output file rather
than `$?`.
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

| `hstego` round trip, greyscale PNG, RGB PNG and JPEG | payload recovered byte-identical on all three |
| `hstego` adaptivity | zero changes in a flat region against 2,224 in a noisy one |

| `outguess` round trip, JPEG | payload recovered byte-identical |
| `stegosuite` round trip, PNG | payload recovered byte-identical |
| A JPEG hidden inside a PNG, and inside a JPEG | both recovered byte-identical |

**All six hiding tools have now had a file put through them and recovered it
byte for byte.** Every carrier they produced still decodes as a valid image.

**Not checked: the detector's accuracy**, and that is the gap that matters
most. On an ordinary photograph `stegcore analyse` flagged the clean original
and both stego files identically. See "Neither verdict means anything on its
own" above.

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

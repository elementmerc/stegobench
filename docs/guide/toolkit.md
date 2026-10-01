# The toolkit image, in detail

`tools/toolkit/README.md` is the short version: how to get the image and how
to run it. This is everything a reader needs once they are actually using it.

## Why the alias looks like that

```sh
alias toolkit='docker run --rm $([ -t 0 ] && printf -- "-it") --network=none --read-only --tmpfs /tmp:rw,noexec,nosuid --security-opt no-new-privileges --cap-drop ALL --user "$(id -u):$(id -g)" -v "$PWD:/data" stegobench/toolkit:latest'
```

Each part is there because leaving it out broke something a real person hit, or
because a platform engineer reviewing the image for a network asked for it.

| Part | Why |
|---|---|
| `--network=none` | None of these tools needs the network. One that suddenly wants it should fail rather than reach out. It is also what makes the build's own network fetches harmless: whatever they brought in cannot reach anything while you run it |
| `--user` | Without it everything the tools write is owned by root, mode 600, and you cannot delete your own output on a shared machine |
| `--read-only` | The tools have no reason to change the image they run from. Nothing here needs a writable root filesystem |
| `--tmpfs /tmp:rw,noexec,nosuid` | The one writable place they do need. openstego and stegosuite run on a JVM that writes to `HOME`, which is `/tmp` here, and numba caches compiled code there. **With `--read-only` and no tmpfs, openstego fails and you can miss it**, which is the next section |
| `--cap-drop ALL` | None of these tools needs a Linux capability. Dropping them also neutralises the stock setuid binaries Debian ships, which are the only ones in the image |
| `--security-opt no-new-privileges` | Stops anything in the container gaining privileges it was not started with |
| `$([ -t 0 ] && printf -- "-it")` | Adds `-it` only when you are actually at a keyboard. Interactive tools need it: the guided wizard exits reporting that you cancelled when you did not, and `steghide` prompts before overwriting and fails with "could not get terminal attributes". But passing `-it` unconditionally breaks every scripted or piped run with `the input device is not a TTY`, so the alias asks rather than guessing |
| `-v "$PWD:/data"` | The tools see your current directory as `/data`. Paths you pass them are paths inside the container |
| the tag | Must match the tag you built. Omit it and docker looks for a `:latest` that may not exist, and reports `pull access denied`, which reads like a login problem and is not one |

**This is measured, not aspirational.** Every one of the six hiding tools was
put through an embed and extract round trip under all of those flags together
on 2026-10-01, and five of the six returned the payload byte for byte. The
sixth is `outguess`, which has a problem of its own that has nothing to do with
these flags; see "`outguess` can hand back the wrong bytes" below. So there is
no feature you give up by running locked down, and no reason to start looser.

### Running as root, if you leave `--user` off

The image starts as root when you do not say otherwise, and it prints a note on
stderr saying so. That is deliberate rather than an oversight: the image cannot
know which user owns the directory you mount, so a fixed non-root default would
leave the tools unable to write their own output in the ordinary case. The note
goes away when you pass `--user`, and `STEGOBENCH_TOOLKIT_QUIET_ROOT=1`
silences it if you are running as root on purpose.

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

## `outguess` can hand back the wrong bytes and report success

Hide a file with `outguess`, get it back, and the file you get can be the right
size with a byte changed in it. Both commands exit 0. Nothing warns you.

Measured on 2026-10-01: the 29 byte payload `hardened profile test payload`
came back as `hardened profile test payloa$`, the final byte changed by a single
bit, the same way on five runs out of five. Three other payloads of 11, 29 and
30 bytes through the same cover and key were perfect, and so were five synthetic
payloads from 8 to 200 bytes.

**That mixture is the whole problem.** It depends on the content, so testing it
once and seeing a clean round trip tells you nothing about the next file. It is
not random: the payload that fails, fails every time.

So if you use `outguess`, check what you extracted against a hash of what you
hid:

```sh
sha256sum secret.txt                  # before
toolkit outguess -k pw -d /data/secret.txt /data/cover.jpg /data/out.jpg
toolkit outguess -k pw -r /data/out.jpg /data/back.txt
sha256sum back.txt                    # must match
```

The exit code will not tell you. The file size will not tell you either.

## `versions` tells you when it cannot answer

`toolkit versions` prints one line per tool and exits 0. If it cannot get a
version out of one of them it prints `COULD NOT DETERMINE` for that tool,
explains the usual cause, and exits 1.

It used to print six lines for seven tools and exit 0, which is worse than an
error: a reader counts the lines they were given, not the ones they were not.
The usual cause is a `--read-only` container with no writable `/tmp`, because
openstego's JVM writes to `HOME`.

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
| `hstego` round trip, greyscale PNG, RGB PNG and JPEG | payload recovered byte-identical on all three |
| `hstego` adaptivity | zero changes in a flat region against 2,224 in a noisy one |
| `outguess` round trip, JPEG | recovered byte-identical on a photograph, and **not** on every payload: see the outguess section above |
| `stegosuite` round trip, PNG | payload recovered byte-identical |
| A JPEG hidden inside a PNG, and inside a JPEG | both recovered byte-identical |
| All of the above under `--user` | clean, output owned by the caller |
| `stegcore`, `steghide`, `openstego`, `hstego`, `stegosuite` under the full locked-down profile | payload recovered byte-identical; `stegosuite` embeds and writes its carrier |
| No vendor-added setuid binary | `find / -xdev -perm -4000 -o -perm -2000` returns the stock Debian set only |
| Nothing phones home | every URL in the `stegcore` binary is an inert constant: XMP namespaces, a dbus spec link, two Rust crate strings and the AUP |

The `openstego` round trip was the one that mattered: it predates Java 21 by
years, and Debian trixie has no `openjdk-17`, so the image runs it on 21. That
it works is a measurement rather than an assumption.

**All six hiding tools have had a file put through them and recovered it byte
for byte**, and five of the six do so under `--read-only --cap-drop ALL
--security-opt no-new-privileges` with a tmpfs `/tmp`. Every carrier they
produced still decodes as a valid image. The exception is not a sandbox problem:
`outguess` returns a wrong byte for some payloads whatever flags you use.

**Not checked: the detector's accuracy**, and that is the gap that matters
most. On an ordinary photograph `stegcore analyse` flagged the clean original
and both stego files identically. See "Neither verdict means anything on its
own" above.

**Pinned, as of 2026-10-01.** Every `ARG *_REF` build argument used to track a
branch, so two builds a week apart could differ. There were five of them, not
the three an earlier version of this paragraph claimed, and all five now name a
commit: `tools/toolkit`, `tools/hstego`, `tools/aletheia`,
`tools/aletheia-rich` and `tools/stegexpose`. The toolkit build also checks
that pip resolved the commit it was asked for and stops if it did not, because
an unchecked pin is a comment.

Two of the five are honest about what was not confirmed: the `aletheia-rich`
resources pin and the `stegexpose` pin could not be read back out of the
already-built images, since neither retains its clone, so they name upstream's
current head on repositories with no commits since 2024 and 2018. The other
three were read out of the built images themselves.

**The openstego `.deb` is pinned by hash.** It is the one file in the build that
comes from a GitHub release rather than Debian's signed mirrors, and a release
asset can be replaced without the version changing. The hash was checked against
the already-built image, where all six files the package installs match dpkg's
own records.

**There is a bill of materials.** `collect_sources.py` writes CycloneDX 1.5
covering all 292 components: 280 Debian packages with purls, the nine Python
packages in hstego's virtual environment, and the five things that belong to no
ecosystem. Those five deliberately carry no purl, because a purl claims an
origin and the whole reason they are listed separately is that they did not come
from the ecosystem they look like they came from. It is reproducible byte for
byte and takes about a minute:

```sh
tools/toolkit/collect_sources.py stegobench/toolkit:latest --out sbom --sbom-only
```

**Outstanding.** An image built outside a git clone cannot name its commit;
`build.sh` now refuses to build one rather than stamping
`revision=not-a-git-checkout`, so pass `STEGOBENCH_VCS_REF` from a machine that
has the clone. A published image also has no registry digest until it is
pushed, and the bill of materials says so rather than leaving the field empty.

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

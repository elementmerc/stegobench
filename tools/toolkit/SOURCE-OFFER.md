# Source offer for the toolkit image

The toolkit image carries programs licensed under the GNU GPL and AGPL. Those
licences say that whoever hands you the binary has to hand you the source it
was built from. This file is how that is done, and the source is built before
the image is published rather than promised and assembled if somebody asks.

## Getting the source

Every published toolkit image has a `toolkit-sources-<tag>.tar.zst` beside it
on the release page, plus a `SOURCES.json` listing what's inside. Download
both. `SOURCES.json` names every package in the image, the version, and the
sha256 of the source file that corresponds to it, so you can check that what
you got is what the binary was built from.

If a link is broken or an archive is missing, open an issue. The offer stands
regardless of whether the download works on the day you try it.

## Which parts of the image this covers

| Component | Licence | Source has to be offered |
|---|---|---|
| steghide | GPL-2.0-only | yes |
| openstego | GPL-2.0-only | yes |
| stegosuite | GPL-3.0-only | yes |
| stegcore | AGPL-3.0-or-later | yes |
| outguess | BSD-3-Clause | no |
| zsteg | MIT | no |
| hstego | MIT | no |
| libjpeg 9e | IJG | no |
| the Debian base | mostly GPL | yes |

**The Debian base is the large part of this**, and it's easy to miss. An image
built on Debian carries a few hundred packages, most of them copyleft, and
every one of those carries the same obligation as the seven tools do. That's
why the collected archive is around a gigabyte for an image whose headline
tools are a few megabytes: 280 packages, 569 source files.

The permissive entries are collected too. They don't have to be, but a reader
asking "what is in this image and where did it come from" should get one
answer rather than two.

## Rebuilding the archive

```sh
python3 tools/toolkit/collect_sources.py stegobench/toolkit:seven --out dist/sources
```

It reads the package list out of the image itself and fetches each source at
the exact version installed, using the image's own apt so the versions can't
drift. Add `--dry-run` to see what it would fetch without fetching it.

It exits 0 only when every package was retrieved. If anything is missing it
says which, and exits 1, because an offer with a hole in it is worse than no
offer: it reads as complete.

**Run it against the image you are about to publish, not a rebuild of it.**
Two builds a week apart can install different package versions, and source
that corresponds to a different build is not corresponding source.

## Three things that bite

**Security updates are a separate archive.** They live on
`security.debian.org` under `trixie-security`, not on the main mirror. A
sources list with only `trixie` and `trixie-updates` silently has no route to
the source of any package carrying a security fix, which is both the package
most likely to be asked about and the one least acceptable to be unable to
supply. The collector reads both.

**A superseded source leaves the mirror quickly.** When a package gets a new
security update, the previous one's source stops being served within days, and
that previous one is exactly what an image built last week is running. The
collector falls back to `snapshot.debian.org`, which keeps every version ever
published. That fallback is addressed by timestamp, so an image carrying a
package newer than the default timestamp needs `--snapshot` moved later.

**AGPL section 13 reaches further than the others.** stegcore is AGPL, so if
you deploy this image in a way that lets people interact with it over a
network, you owe *them* the source too, not just whoever you got the image
from. This image doesn't arrange any network service by itself, so running it
locally doesn't trigger that. Wrapping it in one does.

## Not legal advice

This describes what we do to comply. It isn't advice about what you have to do
if you redistribute the image yourself. Read the licences; they're in
`/usr/share/doc/<package>/copyright` inside the image, one per package.

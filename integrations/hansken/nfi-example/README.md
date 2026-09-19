<!-- SPDX-License-Identifier: Apache-2.0 -->
# Contribution-ready copy for NFI's SDK examples

This directory is a standalone copy of the structural detector, shaped to drop
into `NetherlandsForensicInstitute/hansken-extraction-plugin-sdk-examples` at
`python/steganography/`. It is not used by anything in stegobench; the working
plugin is one level up.

**Nothing here has been sent.**

## Why it is a separate copy rather than a symlink

Licence. stegobench is AGPL-3.0-or-later and their examples repository is
Apache-2.0, so AGPL code cannot be merged into it without imposing copyleft on
their whole collection.

`trailing_data.py` is original work containing no Stegcore code and depending
on nothing but the standard library, so its licence is ours alone to set. It
is dual licensed in the working copy (`AGPL-3.0-or-later OR Apache-2.0`) and
this copy carries the Apache-2.0 header alone, which is the one they can
accept. Everything else, including the whole statistical half and the Stegcore
adapter, stays AGPL and is not offered.

## What is different from the working plugin

| | Working plugin | This example |
|---|---|---|
| Statistical detection | Stegcore's calibrated ensemble | **Removed.** Needs an AGPL binary |
| Dependencies | `stegcore` binary in the image | Standard library only |
| Properties | `stegStructural` plus nine more | `stegStructural`, `stegAssessed`, and three describing what was found |
| Licence | AGPL-3.0-or-later | Apache-2.0 |

Cutting the statistical half is not a loss for an example. It made the plugin
depend on a 9 MB Rust binary built from a pinned commit, which is a packaging
story rather than a teaching one, and it brought a calibration argument that
needs its corpora explained to mean anything.

## The point it is offered to make

Their examples show how to read data, write properties, build child traces,
search, defer and run in bulk mode. **None shows a plugin declining to answer.**

This one writes two properties where most tools write one. `stegStructural`
says what was found; `stegAssessed` says whether the question was asked at
all. A file that does not parse, or is not a picture, or is too large to read,
gets `stegAssessed = no: <reason>` and **no `stegStructural` property at all**,
so it cannot be misread as a clean result.

That matters because `none` and `not assessed` look identical in a result list
and mean opposite things. "We looked and found nothing" closes a line of
enquiry; "we could not look" should open one. Any plugin whose method has a
stated scope has this problem, and it costs one property to get right.

## State

Passes their own framework, 9 of 9.

```
appended_png    stegAssessed = yes   stegStructural = appended data  (3392 bytes at 1033, text)
clean_png       stegAssessed = yes   stegStructural = none
clean_jpg       stegAssessed = yes   stegStructural = none
not_a_picture   stegAssessed = no: not a PNG or JPEG
```

## The fixtures are independent

They are **not** the stegobench fixtures. Those are produced by an AGPL
generator, and putting AGPL-derived test data into an Apache-2.0 repository
would reintroduce the licence problem this package exists to avoid.

`testdata/make_testdata.py` builds these from arithmetic: a synthetic gradient,
a fixed encoder call, a readable note, and an EXIF segment assembled by hand.
It is Apache-2.0, it was written without reference to the stegobench
generator, and the output is deterministic. Two runs produce identical bytes,
verified.

**The JPEG fixture carries a thumbnail, and that is the point of it.** A
camera writes a small copy of the photograph into an EXIF segment, and that
thumbnail is itself a JPEG ending in the same FF D9 bytes as the outer
picture. A plugin that finds the end of a JPEG by searching for those bytes
stops at the thumbnail: on this fixture it would report **7,364 of 8,026
bytes, 91.8% of the file**, as appended to a picture with nothing appended to
it. Without the thumbnail the fixture tests nothing, because a synthetic JPEG
has exactly one FF D9 and the naive search gets it right.

`testdata/verify_testdata.py` asserts every figure quoted above and in the
generator's docstring. It exists because an earlier version of that docstring
quoted numbers written before the fixtures were generated, and they were
wrong.

## Proposed line for their `python/README.md`

> * *AppendedDataDetectionPlugin*: This plugin finds data appended after the
>   logical end of a PNG or JPEG, one of the oldest ways to hide a payload in
>   a picture. It also shows how to distinguish "assessed and found nothing"
>   from "could not assess", so that a file the plugin could not read is not
>   mistaken for a clean one. This plugin can be found in the
>   `python/steganography` directory.

## Before this is sent

- Nobody outside this repository has read it
- See `Stegcore/private/outreach/hansken-plugin-approach.md` for the approach

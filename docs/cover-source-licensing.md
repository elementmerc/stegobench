# Which cover sources can go into a published corpus

Compiled 2026-09-15 by reading the actual terms pages, not by inference. Every
entry below was checked against the source text; where terms could not be
retrieved, that is stated rather than guessed.

This governs one question only: **may we publish stego images derived from these
covers?** Using a corpus to evaluate a detector is a different act with different
rules, and almost everything unpublishable here is still perfectly usable for
that. See "The two corpora" at the end.

## The short version

Nearly the entire academic corpus lineage is closed to us, and the royalty-free
web sources are open. That is not a small inconvenience to route around; it is
the single fact that decides what this dataset can be.

## Can publish derived images

| Source | Basis | Conditions |
|---|---|---|
| **Pexels** | Licence permits modification and use, commercial included | Don't redistribute unaltered photos as a stock resource |
| **Unsplash** | "irrevocable, nonexclusive, worldwide copyright license to download, copy, modify, distribute, perform, and use ... including for commercial purposes" | Same carve-out: no competing stock service |
| **CLIC** | Ships under the Unsplash licence above | As Unsplash |
| **Wikimedia Commons** | "Publication of derivative work must be allowed" and "Commercial use of the work must be allowed" are *entry requirements*, so NC and ND cannot exist there | Per-file: attribution may be required, share-alike may be required |
| **Open Images** | Images listed as CC BY 2.0; already ships per-image URL, licence and MD5 | Attribution. Google explicitly disclaims warranting each image's status, so verification is ours |

All five permit commercial use. That matters here specifically: Stegcore is
dual-licensed with a commercial tier, so any non-commercial restriction is a
standing argument waiting to be had.

## Cannot publish derived images

| Source | Why | Verbatim |
|---|---|---|
| **ALASKA / ALASKA2** | CC BY-NC-ND. The ND term is decisive | "this license explicitly forbids ... the distribution of any material build upon the material provided, especially if you remix and transform the dataset" |
| **BOSSbase** | Licence file is on a dead host and was never archived. The live download host states the opposite of a grant | "may not be distributed or republished in any form or by any means ... without the prior express written permission" |
| **Dresden** | Derivatives *are* permitted, but non-commercial only | "You may not use or distribute the data or any derivative work for commercial purposes" |
| **RAISE** | Same licence text as Dresden, names swapped | "to be used for non-commercial research and educational purposes" |
| **BOWS2** | Permits redistribution, non-commercial only, and the licence names the *watermarked* set rather than the originals steganalysis actually uses | "You may not use this work for commercial purposes" |
| **DIV2K** | No redistribution grant exists to rely on | "academic research purpose only ... the copyright belongs to the original owners" |
| **LAION** | Images were never LAION's to license | "The images are under their copyright" |
| **MIRFLICKR** | Mixed CC including ND. A stego image is a modification, and ND bars distributing one | Per-image licence metadata ships, so a BY/BY-SA-only subset is mechanically possible |
| **IStego100K** | No licence of any kind in the repository | Citation request only |
| **StegoAppDB** | No terms ever published, and the host is gone | — |
| **LSSD** | Derivative of six upstream corpora including ALASKA and StegoAppDB, inheriting the worst terms of each | Citation obligation only |
| **UCID**, **USC-SIPI** | No grant. USC-SIPI says outright it cannot give one | "USC-SIPI does not hold the copyright status on many of the images ... we are not in a position to grant such permission" |

## Three things worth knowing before publishing anywhere

**A manifest does not rescue a use restriction.** Shipping a list of URLs plus a
downloader removes the copyright question for the pixels, because we distribute
none. It does nothing about a term that binds the *recipient*, which is what
DIV2K's "academic research purpose only" is. The pattern is narrower than its
reputation.

**The manifest pattern has failed in practice.** WebVid distributed URLs and
captions only, took a cease-and-desist from Shutterstock, and was gone in a day
without the legal question ever being reached. ImageNet's own terms make users
indemnify it against "copies of copyrighted images that he or she may create from
the Database", which is an admission that the downloader produces the infringing
copy. LAION took down the *list* after the Stanford report, not merely the
images. Every party involved has behaved as though the manifest carries the
liability of the thing it points at.

**Third-party licence labels are not evidence, and this is not a one-off.** Two
independent examples found while checking eleven sources:

- **Dresden** is mirrored on Kaggle under "CC0: Public Domain". Its upstream
  licence forbids commercial use and requires the copyright notice to survive
  into derivatives.
- **UCID** is mirrored on Kaggle under "MIT". Its original distribution stated no
  licence at all, so there was nothing for an uploader to relabel *from*.

Both are uploader assertions with no basis upstream. Anyone relying on either
field to justify commercial use is relying on a stranger's mistake. When this
corpus is published, its licence field must be accurate to the source, per file,
or it becomes the third such trap.

## The two corpora

Keep them separate, because they answer to different rules.

- **The evaluation corpus** may use anything we are licensed to *use*. Running a
  detector over ALASKA2 or BOSSbase is use, not redistribution, and nothing here
  prevents it. This is where head-to-head measurement happens.
- **The published corpus** uses only the five sources in the first table. It is
  smaller to assemble and it is the one that can actually be handed to anyone.

That split costs nothing and preserves every result already measured.

## Why this is the opportunity rather than the obstacle

Every well-known steganalysis corpus is either non-commercial, no-derivatives,
unlicensed, or hosted on a machine that no longer answers. BOSSbase's licence
file, Dresden's entire site, StegoAppDB's domain and BOWS2's original host are
all gone. The field has no corpus that a commercial project can lawfully build
on and redistribute.

Building one from sources that plainly permit it would be the first of its kind,
and that is a more durable differentiator than image count. The reason nobody has
is not that it is hard; it is that the academic corpora were free enough for
academic work and nobody needed to ask the question.

## Quality traps in the older corpora

Licensing is not the only reason to prefer fresh sources. Two of the corpora
above carry defects that bite a steganalysis experiment specifically, and neither
is advertised:

- **USC-SIPI's Brodatz textures have rows of literal zeros.** The pigskin images
  (1.1.11, 1.2.11) are missing their last 26 lines; the rest of the 512x512
  Brodatz set is missing its last two. A band of zeros is a degenerate input to
  any least-significant-bit statistic, so a detector will produce a meaningless
  estimate on those images rather than a wrong one.
- **UCID is downscaled, and does not say so.** Every file carries
  an ImageMagick `Software` tag from 2002 and the source camera is a roughly 3.3 MP
  Minolta DiMAGE 5, so 512x384 is a heavy resample. Neither the paper nor the
  project page mentions it. Resampling averages neighbouring pixels, which is
  exactly the statistic spatial steganalysis reads, so UCID is not a
  sensor-native cover source however uncompressed its TIFFs are. A further 39 of
  its 1,338 files are PackBits-compressed rather than uncompressed and carry no
  camera tags, which will trip any pipeline assuming a uniform format.

Neither defect is fatal for casual use. Both are the kind of thing that produces
a confusing result weeks later rather than an error at load time.

## Outstanding

- **Ask the ALASKA authors.** Their licence says "without prior authortisation
  from the authors", which means authorisation is a thing that can be given.
  One email.
- **Cassavia** (Kaggle `marcozuppelli/stegoimagesdataset`) has not been checked.
- **BOSSbase** would move from "cannot" to "unclear" if anyone can produce the
  original `LicenceBOSSBase.txt`. It is worth one email to the DDE lab.

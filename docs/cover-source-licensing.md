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
| **Pexels** | **DISPUTED, see below.** The Licence permits modification; the Terms of Service ban the acquisition method | Do not build a new arm from it until resolved |
| **Unsplash** | "irrevocable, nonexclusive, worldwide copyright license to download, copy, modify, distribute, perform, and use ... including for commercial purposes" | Same carve-out: no competing stock service |
| **CLIC** | Ships under the Unsplash licence above | As Unsplash |
| **Wikimedia Commons** | "Publication of derivative work must be allowed" and "Commercial use of the work must be allowed" are *entry requirements*, so NC and ND cannot exist there | Per-file: attribution may be required, share-alike may be required |
| **Open Images** | Images listed as CC BY 2.0; already ships per-image URL, licence and MD5 | Attribution. Google explicitly disclaims warranting each image's status, so verification is ours |

All permit commercial use. That matters here specifically: Stegcore is
dual-licensed with a commercial tier, so any non-commercial restriction is a
standing argument waiting to be had.

### Pexels: the Licence and the Terms of Service disagree

The Pexels **Licence** permits modification and bars only redistribution "on
other stock photo or wallpaper platforms", which is what this document said
first. The Pexels **Terms of Service** separately ban "Bulk, large-scale or
systematic copying of Content ... unless explicit permission has been granted by
us" and "the use of programs or robots for automatic data collection ...
including without limitation for machine learning purposes".

`generators/fetch_pexels.py` is a program that systematically collects content in
bulk to build a corpus. The Licence governs what may be done with an image; the
Terms govern how it may be obtained, and the acquisition is the half in doubt.

**This affects the 200 covers already fetched**, so it is not merely a rule for
the next arm. Two ways out, and they are not exclusive: ask Pexels for the
explicit permission the Terms contemplate, or rebuild the web arm from
**Unsplash under the ordinary Unsplash Licence**, which is the most permissive
text in this entire survey and carries no equivalent acquisition clause.

One trap worth naming: the **Unsplash Dataset** ships under separate terms that
forbid publishing any portion of it. Only the ordinary route works.

### The Unsplash Dataset, read in full on 2026-09-16

The repository at `github.com/unsplash/datasets` looks like the answer to the
cover-acquisition problem and is not. Three separate things share the Unsplash
name and only the middle one is open to us.

| Thing | Ruling |
|---|---|
| **The Dataset** (`unsplash/datasets`, TSV files) | **Out.** "It cannot be used to redistribute the images contained within." Redistribution of the Licensed Data "in whole or in part" needs written permission, and the Lite grant is narrowly "internally use the Commercial Licensed Data to train machine learning models or algorithms for your internal business purposes". The Full set of 7.4M photos is "non-commercial usage only", which collides with Stegcore's commercial tier |
| **The ordinary Unsplash Licence** (photos from the site or API) | **In**, as recorded above |
| **The API Terms** | **Workable.** No retention limit and no ban on storing copies. It asks for download-event notification, preserved attribution, and staying inside quota |

Two clauses are worth carrying forward even though the Dataset is out. Using API
content "in connection with any machine learning and/or artificial intelligence
purposes" is directed to a separate data licence, and a detection corpus is
squarely that, so it is a question to put in writing rather than assume. And
publishing "the results of any comparison of the Datasets or Licensed Data to
similar datasets" requires written permission, which is precisely what a corpus
paper does.

The Dataset was still worth reading. Its `photos.tsv` carries `exif_camera_make`,
`exif_camera_model`, `exif_iso`, `exif_exposure_time` and `exif_aperture_value`,
which named the axis this corpus was missing: **acquisition diversity**. The API
returns the same fields per photo. So did Commons, as it turned out, for free.

### Flickr: the same shape of conflict as Pexels

The per-photo Creative Commons grant comes from the photographer and is
irrevocable, so redistribution of a CC BY or CC BY-SA Flickr photo is plainly
permitted. The **Flickr API Terms** separately forbid an application to "cache or
store any Flickr user photos other than for reasonable periods in order to
provide the service you are providing to Flickr users". A permanent research
corpus is not a reasonable period.

Licence says yes, acquisition method says no: identical to Pexels. The route that
works is the one already in use, since Commons holds a very large quantity of
CC-licensed Flickr photography, already licence-vetted at upload, with no
equivalent acquisition clause.

**The selection rule this produces: prefer hosts that publish their own bulk
access.** A host that ships dumps and documents its API for reuse cannot
simultaneously forbid collecting in bulk, and that single test separates Commons
from Pexels, Flickr and the Unsplash Dataset without needing to parse each one.

## Cannot publish derived images

| Source | Why | Verbatim |
|---|---|---|
| **ALASKA / ALASKA2** | CC BY-NC-ND. The ND term is decisive | "this license explicitly forbids ... the distribution of any material build upon the material provided, especially if you remix and transform the dataset" |
| **BOSSbase** | Was never permissively licensed. The BOSS organisers claimed rights explicitly and required assent before download; the licence file itself is on a dead host and was never archived | "The organisers hold the rights on the ... contents ... provided for the BOSS contest" and "any use of the BOSS image-database will have to be done according to a licence usage agreement" |
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

**Licence laundering is systemic here, not incidental.** Mirrors of these
corpora carry labels their uploaders had no standing to grant, and it is the norm
rather than the exception:

- **BOSSbase** mirrors are labelled `MIT`, `Apache 2.0`, `CC0: Public Domain` and
  `Unknown` across a dozen re-uploads. Not one traces to a grant from the rights
  holders, who stated plainly that they held the rights.
- **ALASKA2** mirrors are labelled `CC0: Public Domain` while the organisers'
  own page says CC BY-NC-ND. That upstream licence is explicit, stated in plain
  words, and trivially checkable, and it is contradicted anyway.
- **Dresden** is on Kaggle as `CC0: Public Domain`; upstream forbids commercial
  use and requires the copyright notice to survive into derivatives.
- **UCID** is on Kaggle as `MIT`; its original distribution stated no licence at
  all, so there was nothing to relabel *from*.

Any survey that reports mirror licences at face value propagates the error. When
this corpus is published, its licence field must be accurate to the source, per
file, or it joins the list.

## The prior art that moved while we were not looking

**REVEAL**, Netherlands Forensic Institute and University of Amsterdam, October
2025. 100,006 base images from 57 imaging methods, run through **51 real
end-user steganography tools** at **10 payload rates down to 0.00001 bpp**,
paired, **CC BY-SA 4.0**, downloadable today.

Assume a reviewer knows it. Until it appeared, "real tools rather than reference
implementations" was the obvious pitch for a new corpus; that ground is taken.

What it deliberately leaves open, in its authors' own words, is that it **excludes
academic content-adaptive schemes entirely** ("we exclude these schemes from our
dataset because we believe this comprises a much smaller part of steganography in
the wild"). It also covers only png, jpg, bmp and gif, and its tool snapshot is
frozen at July 2023, a limitation it states about itself: "REVEAL will be highly
sensitive to software versioning."

So the unoccupied position is narrower and sharper than it was: **one corpus
spanning real tools AND adaptive academic schemes AND spatial LSB at camera
diversity AND very low payload, as separate arms rather than a blend.** Nobody
holds that.

Two other entrants worth knowing. **StegBench** (August 2026) claims 525,000
images across six tools and has **shipped no images at all**; its repository
holds a README and nothing else. And the name is a problem: "StegBench" now
refers to that paper, to a 2021 MIT-licensed tool, and to a gated LLM
covert-channel corpus. "stegobench" is one letter away from a collision with a
paper five weeks old.

## The second differentiator: these corpora are recipes, not artefacts

The version of BOSSbase that most deep-learning steganalysis actually uses does
not exist as a file anyone published. It is a procedure: resize 512x512 to
256x256 with Matlab `imresize` at default settings, optionally recompress at
quality 75 or 95, then split 14,000 train / 1,000 validation / 5,000 test. That
recipe is stated precisely in the SRNet paper and reproduced from YeNet before
it.

Nobody ships checksums for the result. `imresize` defaults are a Matlab version
dependency that no paper records. So two groups reporting on "BOSSbase 256" may
be reporting on different bytes, and there is no way to tell.

The decay is visible in the wild. One Hugging Face redistribution advertises
20,000 covers plus 20,000 WOW and 20,000 S-UNIWARD stego images as PNG; it
actually holds 9,975 covers, 5,475 WOW and 750 S-UNIWARD, all PGM, drawn from
about 5,000 distinct source images rather than 10,000, with no cover-to-stego
pairing file. It is MIT-labelled and has over a thousand downloads.

Every generator in this repository is seeded and writes a manifest with a sha256
per file and the count of samples actually changed. That is not a nicety; it is
the property the field is missing, and it is cheaper to have from the start than
to retrofit.

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

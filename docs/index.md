---
layout: home

hero:
  name: Stegobench
  text: Steganalysis numbers somebody else can check
  tagline: Build a labelled corpus, run detectors over identical bytes, and report results a stranger can reproduce.
  actions:
    - theme: brand
      text: What it is
      link: /guide/what-it-is
    - theme: alt
      text: Quickstart
      link: /guide/quickstart
    - theme: alt
      text: Source
      link: https://github.com/elementmerc/stegobench

features:
  - title: Pairing enforced, not described
    details: A clean image and its stego twin come off the same encoder. When they did not, a whole round of measurements here turned out to be detecting the resave rather than the payload.
  - title: Scores kept, not thresholded away
    details: Several detectors compute an estimate, compare it to a threshold and print a sentence. You cannot draw a curve from sentences, so where a tool discards its number the harness calls the same function and keeps it.
  - title: Controls you can run against your own result
    details: Arms that carry no meaningful payload, so an effect you measure can be checked against one that cannot exist.
  - title: Rebuildable byte for byte
    details: Every generator is seeded, every file carries a sha256, and every sample records how many pixels or coefficients actually changed.
---

## Start here

| If you want to | Read |
|---|---|
| Know what this is before installing anything | [What it is](/guide/what-it-is) |
| Run it | [Quickstart](/guide/quickstart) |
| Build a corpus | [Build a corpus](/guide/build-a-corpus) |
| Understand what makes a measurement valid | [Pairing](/guide/pairing) |
| Know what it does not do | [Limitations](/guide/limits) |

Looking for the corpus rather than the harness? That is
[Pentimento](/pentimento/), documented separately.

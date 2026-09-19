---
layout: home

hero:
  name: stegobench
  text: Steganalysis numbers somebody else can check
  tagline: Build a labelled corpus, run detectors over identical bytes, and publish results with the corpus attached.
  actions:
    - theme: brand
      text: What it is
      link: /guide/what-it-is
    - theme: alt
      text: The corpus
      link: /pentimento
    - theme: alt
      text: Source
      link: https://github.com/elementmerc/stegobench

features:
  - title: A corpus you are allowed to redistribute
    details: 10,000 cover photographs from Wikimedia Commons, every one carrying its own licence, and 344,348 matched stego pairs that inherit it. The academic corpora this field runs on mostly cannot be republished at all.
  - title: Pairing enforced, not described
    details: A clean image and its stego twin come off the same encoder. When they did not, a whole round of measurements here turned out to be detecting the resave rather than the payload.
  - title: Scores kept, not thresholded away
    details: Several detectors compute an estimate, compare it to a threshold and print a sentence. You cannot draw a curve from sentences, so where a tool discards its number the harness calls the same function and keeps it.
  - title: Rebuildable byte for byte
    details: Every generator is seeded, every file carries a sha256, and every sample records how many pixels or coefficients actually changed.
---

## Start here

| If you want to | Read |
|---|---|
| Know what this is before installing anything | [What it is](/guide/what-it-is) |
| Run it | [Quickstart](/guide/quickstart) |
| Build your own corpus | [Build a corpus](/guide/build-a-corpus) |
| Use the published one | [Pentimento](/pentimento) |
| Quote a number from it | [Read this first](/guide/limits) |

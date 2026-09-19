---
layout: home

hero:
  name: Pentimento
  text: A steganalysis corpus with its licences attached
  tagline: 10,000 cover photographs and 344,348 matched stego pairs, every image carrying its own licence and every file its own checksum.
  actions:
    - theme: brand
      text: Get it
      link: /guide/get-it
    - theme: alt
      text: What is in it
      link: /guide/whats-in-it
    - theme: alt
      text: Limitations
      link: /guide/limitations

features:
  - title: You are allowed to redistribute it
    details: Built from permissively licensed Wikimedia Commons photographs. The standard research corpora in this field either carry no readable licence or forbid publishing anything derived from them.
  - title: The credit line travels with the pixels
    details: 5,429 of the covers require attribution, and their stego derivatives inherit it. Every sample carries its cover's licence, artist and source URL, so one arm is enough to comply.
  - title: Pairs that differ only in the payload
    details: Both halves of every pair come off the same encoder. Where a tool rewrites the file, its clean half is written by that same tool.
  - title: Rebuildable byte for byte
    details: Every generator is seeded, every file has a sha256, and every sample records how much actually changed.
---

## Start here

| If you want to | Read |
|---|---|
| Know what this is | [What it is](/guide/what-it-is) |
| Download it | [Get it](/guide/get-it) |
| Know what is inside | [What is in it](/guide/whats-in-it) |
| Train on it | [Loading and splitting](/guide/using-it) |
| Publish a number from it | [Limitations](/guide/limitations) |
| Comply with the licence | [Licence and attribution](/guide/licence) |

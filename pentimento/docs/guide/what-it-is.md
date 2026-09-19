# What it is

Pentimento is a labelled corpus for measuring steganalysis: clean photographs
paired with copies of themselves that carry a hidden payload.

Steganalysis is the business of looking at a picture and deciding whether
somebody hid a message inside it. To measure how well a detector does that, you
need images where you already know the answer, and you need a lot of them.

## Why another one

The corpora this field runs on cannot be redistributed.

| Corpus | What it permits |
|---|---|
| BOSSbase | No readable licence survives. The organisers claimed rights and required assent before download; mirrors advertising CC0, MIT and Apache are all wrong |
| ALASKA2 | CC BY-NC-ND. The no-derivatives term forbids distributing anything built from it, which is exactly what a stego image is |

Both are fine to *measure* on. Neither can be republished, so a reader who wants
to check a published result cannot obtain the data it was measured on.

Pentimento is built from photographs whose licences permit exactly that, and
each file's own terms travel with it.

## What it is not

It is not a benchmark leaderboard. It is data; what you measure with it is up to
you.

It is not comparable to BOSSbase. See [Limitations](/guide/limitations), which
is the first thing to read before quoting a number from it.

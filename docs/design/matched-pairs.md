# Matched pairs, and the defect class that kept getting past us

This is a design note for contributors. It describes a kind of bug that has now
broken two separate arms of the corpus, explains why every check in place at
the time passed anyway, and documents the check that catches it. The
user-facing summary is [Pairing, and what breaks it](../guide/pairing.md); this
page is the long version, including the part where the first attempt at a fix
was itself wrong.

## What a pair is supposed to be

A steganalysis corpus is made of pairs. One image is clean. Its twin carries a
hidden payload. A detector is shown both and asked which is which, and the
score is how often it gets that right.

That question only measures hiding if **the payload is the only difference
between the two files**. Any other difference is an easier question sitting
next to the hard one, and a detector will answer the easier one without telling
you it did. The score still looks like a steganalysis result. It is a result
about whatever else changed.

```
    cover ──► [ payload ] ──► stego half   the ONE difference
    cover ─────────────────►  clean half

    cover ──► [ payload ] ──► [ writer A ] ──► stego half   two differences,
    cover ────────────────►   [ writer B ] ──► clean half   and a detector
                                                            takes the cheap one
```

## Instance one: two different encoders

The outguess arms were built by handing the tool a cover and keeping what it
wrote. Outguess re-encodes the whole JPEG, so the stego half came off outguess's
encoder. The clean half came off the encoder that produced the cover. Two
different pieces of software wrote the two halves of every pair.

The measured result:

| Payload | AUC |
|---|---|
| One single byte | 0.360 |
| 0.03% of capacity | 0.3602 |
| 50% of capacity | 0.3598 |

A payload effect grows with payload. This one was flat across a 1,600-fold
increase in payload, and it was **below** 0.5, which means the classifier was
reliably picking the wrong one: the stego half was the smoother of the two,
because of how outguess rounds, not because of anything hidden in it.

One byte of payload and half the file's capacity gave the same answer to three
decimal places. That is the signature of a measurement that has nothing to do
with the payload.

## Instance two: the same encoder, one extra pass

The fix for instance one was to pair every tool against its own writer. That is
correct as far as it goes, and it is not far enough.

The J-UNIWARD and UERD arms, eight arms and 80,000 images, were built so that
both halves went through `jpeglib`. Same library, same version, same settings.
The pairing check at the time asked "did the same writer produce both halves?"
and the honest answer was yes.

They still separated perfectly, for this reason:

```
    clean half:  cover ──► [ jpeglib write ] ──────────────────────► shipped
    stego half:  cover ──► [ jpeglib write ] ──► [ embed, write ] ──► shipped
                                                        ^^^^^
                                        one more pass through the writer
```

The stego half was produced by reading the clean JPEG, changing coefficients,
and writing it back. That is one extra `jpeglib` write. `jpeglib` prepends a
JFIF APP0 segment every time it writes, so:

| Half | APP0 segments |
|---|---|
| Clean | 2 |
| Stego | 3 |

Measured on 200 pairs in each of three arms: 600 pairs, zero matching headers.
Counting APP0 segments labels every image in those arms correctly. No
steganalysis, no features, no classifier. Count a marker and read off the
answer.

## Why nothing caught either one

This is the part worth sitting with, because the controls were not weak.

| Check | What it said | Why it did not help |
|---|---|---|
| Per-file sha256 | Every file matched its recorded digest | The file was intact. It was also wrong |
| Licence join | Every sample traced to a licensed cover | Nothing to do with pairing |
| Sample counts | Every arm had the expected number | Nothing to do with pairing |
| Pre-registered null arm | Passed | It tested the tool's re-encode, not the extra pass |
| "Same writer" check | Passed, and was TRUE | It was the wrong question |

Every one of those checks read the image content, or read metadata about the
image content. **The difference in both instances was not in the content.** It
was in the container: the bytes around the image data that say how to decode
it. Nothing looked there.

The "same writer" check deserves its own line, because it is the instructive
failure. It did not malfunction and it did not return a wrong answer. It
answered its question correctly and its question was insufficient. Passing the
wrong check feels exactly like passing the right one.

## The check that catches it

`container_of()` in
[`generators/pack_arms.py`](../../generators/pack_arms.py) reduces a file to
everything about it that a payload has no business changing, and the gate in
`pack_arm()` refuses any pair whose two halves disagree.

| Format | What is compared |
|---|---|
| JPEG | Every marker and its length before the start of scan: quantisation tables, Huffman tables, the frame header, every application segment |
| PNG | The sequence of chunk types, plus the length of every chunk that is not image data |
| Anything else | Nothing, and it says so by returning an empty tuple |

The last row matters. A format the function does not understand must not be
silently declared matched, and must not be silently declared broken either. It
returns "no opinion", and every format the corpus actually carries is handled
above it.

Every sample that survives the gate records `pairing: container-verified`. An
absent field used to be ambiguous between "checked and matched" and "nobody
looked", and that ambiguity is precisely where both of these defects lived.

## The trap in the fix

The first version of `container_of` refused 1,863 pairs that were completely
sound.

What gave it away was not the number but its shape:

| Payload rate | Pairs refused |
|---|---|
| 0.05 bpp | 18 |
| 0.4 bpp | 218 |

The false-positive rate climbed with the treatment. That is the tell, and it is
worth naming as a general diagnostic:

> **If your false-positive rate tracks the treatment, your check is reading the
> treatment.**

The cause: Pillow splits PNG image data into IDAT chunks at 64 KiB. A payload
makes the pixels less compressible. Less compressible pixels mean more
compressed bytes, and enough extra bytes mean one more IDAT chunk. A cover
whose pixels compress to 60,179 bytes ships one IDAT; its stego twin at 67,817
ships two.

That extra chunk is the payload doing its job. It is not the writer leaving a
mark. So a run of IDAT chunks now collapses to a single entry: where the image
data sits relative to every other chunk still has to match, but how many pieces
it arrives in does not, and neither does its compressed length.

The general form of that lesson is the harder half of building this check:

> **A structural check has to exclude everything the payload is entitled to
> change, and that list is longer than it looks.**

A check that is too strict is not the safe side of the error. It rejects good
data, and it rejects the most heavily loaded samples first, which quietly
reshapes the corpus towards low payloads: the exact region where detection is
hardest and the numbers are most flattering.

## The lesson, in one paragraph

Both instances were caught by somebody comparing two files byte by byte and
asking what differed. Neither was caught by a check, and in the second case a
check that looked directly at the problem passed, because it asked whether the
same writer produced both halves and it did. The right question is not who
wrote each half. It is **what guarantees that both halves went through an
identical pipeline**, and the way to answer it is to compare the two files
structurally rather than to reason about the code that produced them.

When you add an arm, assume this class of defect is present until a structural
comparison says otherwise. It has been there twice, and both times everything
else looked fine.

# REVEAL: the corpus to calibrate against, and the one to beat

Kombrink, van Lierop, Stolwijk, Worring, Vrijdag and Geradts. Netherlands
Forensic Institute and University of Amsterdam. *Forensic Science International:
Digital Investigation* 55 (2025), article 302006. Accepted 25 September 2025,
online 29 October 2025.

| | |
|---|---|
| **Paper DOI** | `10.1016/j.fsidi.2025.302006` |
| **Open copy** | `https://pure.uva.nl/ws/files/273178780/1-s2.0-S2666281725001465-main.pdf` |
| **Dataset** | **`https://doi.org/10.17026/PT/DITX0A`** → DANS Physical and Technical Sciences Data Station. Verified resolving 2026-09-16 |
| **Code** | `https://github.com/NetherlandsForensicInstitute/REVEAL`, GPL-3.0, last pushed 2026-07-16 |
| **Contact** | `m.kombrink@nfi.nl` |
| **Funding** | EU Horizon 2020, grant 101021687, project UNCOVER |

Note the two licences differ: the **code is GPL-3.0**, the **dataset is CC BY-SA**.

## What it contains

100,006 base images from more than 50 cameras, with a deliberately wide spread of
attributes: over 200 distinct sizes from 256x256 to 7680x4320. Those are put
through chains of 41 preprocessing options, then through **more than 50
steganography algorithms**, producing three sets (original, preprocessed, stego)
totalling more than 300,000 images. Paired by construction, and it refuses to
embed one image with several tools, in the authors' words, "in order to avoid
bias based on specific images".

It is the first steganography dataset to use many widely available end-user
software packages rather than reference implementations, and the first to include
AI-generated images alongside camera images.

## Licence, quoted

> "This dataset may be used freely under the CC-BY-SA license. Hence the dataset
> is free to use for anyone (both commercial and academic use are allowed),
> though it does require attribution and mandates that any new works derived
> from the dataset must be shared under the same or a compatible license."

**Commercial use is allowed**, which makes REVEAL almost unique among the corpora
surveyed. The sting is share-alike: anything derived from it must also be
CC BY-SA. That is why it belongs in its own arm rather than blended into a corpus
we want to licence on our own terms, and why using it to *calibrate* is free of
that problem while using it as *source material* is not.

## Using it to calibrate Stegcore

This is the more immediate value, and it carries no licensing cost at all.
Running a detector over a corpus is use, not redistribution, so nothing about
share-alike bites.

What makes REVEAL unusually good for this:

- **Over 50 real end-user tools.** Stegcore's thresholds were calibrated against
  Cassavia, BOSSbase and an ALASKA2 sample, all of which are reference
  implementations or research pipelines. REVEAL is what people actually run.
- **Payload rates to 0.00001 bpp.** Stegcore's documented weak spot is p <= 0.1,
  and the published grids in the field bottom out around 0.05. REVEAL goes four
  orders of magnitude lower, which is where S12 (the calibration ceiling found on
  2026-09-15) can actually be characterised rather than guessed at.
- **Over 200 image sizes, up to 7680x4320.** Every Stegcore calibration figure to
  date rests on 512x512.
- **AI-generated covers.** An arm nobody else has, and a distribution any deployed
  detector now meets.

The obvious first experiment is the one that has never been run: Stegcore's
per-detector thresholds (SPA 0.377, RS 0.305, WS 0.195) and its verdict ladder
against a corpus built from tools it has never seen, reported per tool and per
payload rate.

## The full tool list

Pulled from `ToolsRunInfo.xlsx` in the REVEAL repository on 2026-09-18, which is
the authoritative inventory. The repository's README does not list the tools and
says plainly that they "have not been included in this repository and can be
downloaded separately", so this table is the only place the set is enumerated
outside a spreadsheet.

Note the count. The paper's abstract says "more than 50 steganography
algorithms" and the spreadsheet holds **51 stego tools** plus a control
condition. Several of our earlier notes said "over 50 real end-user tools",
which is right, but it is worth being exact now that the list is to hand.

**51 stego tools**, plus one control condition (no embedding), 52 rows in total.

Run OS: Windows 27, Linux 24

Implementation: Windows 22, Python 10, Linux 5, Go 3, C 2, Java(script) 2, C# 2, Online 1, Python 2 1, C++ 1, Java 1, Rust 1

9 require a key. 1 are flagged by the authors as **especially difficult to find**.

| ID | Tool | Algorithm | Impl | OS | Version | In | Out |
|---|---|---|---|---|---|---|---|
| 0 | Control Condition | None | - | Windows | - | png-jpeg-gif-bmp-jpg | png-jpeg-gif-bmp-jpg |
| 1 | LSB Replacement Stegote | LSBR | Python | Linux | Commit Sep 3 2019 | jpeg-jpg-png | png |
| 2 | StegArmory PVD | PVD | Python | Linux | Commit Oct 26 2021 | png | png |
| 6 | Steghide | Graph theoretic approach | Linux | Linux | 0.5.1 | jpeg-bmp | jpeg-bmp |
| 8 | Stegify | LSB | Go | Linux | 1.2 | png-jpeg-jpg | png |
| 10 | SteganographX Plus | Unknown | Windows | Windows | V2.0 | bmp | bmp |
| 12 | JSteg | LSB | Go | Linux | 0.3.0 | jpeg-jpg | jpeg-jpg |
| 13 | SSuite Picsel | Unknown | Windows | Windows | Downloaded: Oct 30 2022 | png-jpeg-bmp-jpg | png-bmp |
| 14 | Our Secret | Unknown | Windows | Windows | v2.5 | png-jpeg-gif-bmp-jpg | png-jpeg-gif-bmp-jpg |
| 15 | Stegano | LSB | Python | Linux | 0.10.1 | png-jpeg-gif-bmp-jpg | png-jpeg-gif-bmp-jpg |
| 17 | Open Stego | RandomLSB | Windows | Windows | v0.8.5 | png-jpeg-gif-bmp-jpg | bmp-png |
| 18 | SteganPEG | Unknown | Windows | Windows | 1.0 | jpeg-jpg | jpeg-jpg |
| 19 | Steganography / Stegolsb | LSB | Python | Linux | 1.3.4 | jpeg-jpg | jpeg-jpg |
| 20 | Hide N Send - M-F5 (default) | M-F5 | Windows | Windows | v1.02 | jpg | jpg |
| 21 | Stego | Unknown | Linux | Linux | 0.1.4 | png | png |
| 25 | stegman | Appends data | C | Linux | v1.0.0 | png | png |
| 26 | Tartarus | Hides in metadata | Python | Linux | Commit May 16 2021 | png-jpeg-bmp | png-jpeg-bmp |
| 29 | StegOnline - MSB (default), selecting all 0 level bits | MSB | Online | Windows | Website | jpeg-jpg | jpeg-jpg |
| 30 | quickstego | Unknown | Windows | Windows | Downloaded: Oct 30 2022 | jpeg-bmp-gif | bmp |
| 31 | Openpuff - PNG default quality preset | Unknown | Windows | Windows | v4 | png | png |
| 42 | DCT-Image-Steganography | DCT | Python | Linux | Commit Dec 18 2020 | jpeg-jpg | jpeg-jpg |
| 43 | Matroschka | LSB | Python 2 | Linux | Commit Nov 9 2019 | png-bmp | png-bmp |
| 45 | Stepic | LSB | Linux | Linux | 0.4.1 | png-jpeg-bmp-jpg | png-jpeg-bmp-jpg |
| 49 | Pixel Value Difference | PVD | Python | Linux | Commit Jun 25 2022 | png-jpeg-gif-bmp-jpg | png-jpeg-gif-bmp-jpg |
| 52 | Paranoia | F5 | Java(script) | Windows | v15.0.3 Multi Platform Desktop (Java) | png-jpg-bmp | jpg |
| 54 | JPHIDE-JPSEEK | Unknown | C++ | Linux | v0.3 | jpeg-jpg | jpeg-jpg |
| 55 | SecurEngine | Unknown | Windows | Windows | v4 | png-gif-bmp | png-gif-bmp |
| 58 | WBStego | Unknown | Windows | Windows | Last Updated: Mar 15 2003 | bmp | bmp |
| 59 | 2Pix | LSB | Windows | Windows | Last Update: 2015-04-24 | png-jpeg-gif-bmp-jpg | bmp |
| 60 | aleXaSteganographyTool | Unknown | C# | Windows | Last Update: 2016-12-27 | png-jpeg-gif-bmp-jpg | png-jpeg-gif-bmp-jpg |
| 64 | HIDEAndREVEAL | LSB | Java(script) | Windows | v1.7.0 | png-bmp | png-bmp |
| 65 | Steganography Studio - BattleSteg Algorithm | BattleSteg | Windows | Windows | v1.0.2 | png-jpg-bmp | png-bmp |
| 67 | steganography PNG | Hides in IDAT field | Go | Linux | Commit Dec 5 2021 | png | png |
| 79 | Hide4PGP | Unknown | Linux | Linux | v2.0 | bmp | bmp |
| 102 | OpenStego | Unknown | Windows | Windows | 0.8.5 | png-jpeg-bmp-jpg | png-jpeg-bmp-jpg |
| 104 | invisible-watermark | DWT + DCT | Python | Linux | v0.1.5 | png-jpeg-bmp-jpg | png-jpeg-bmp-jpg |
| 105 | OpenPuff | Unknown | Windows | Windows | v4.0.0 | png-jpeg-bmp | png-jpeg-bmp |
| 126 | LSB Matching Stegote | LSBM | Python | Linux | Commit Sep 3 2019 | jpeg-jpg-png | png |
| 149 | Stegomatic | Graph theoretic approach | C# | Windows | Commit Feb 27 2019 | png-jpeg-bmp-jpg | png-bmp |
| 159 | Stegosuite | Unknown | Linux | Linux | - | png-gif-bmp-jpg | png-gif-bmp-jpg |
| 161 | StegoShare | LSB | Windows | Windows | v1.01 | png-jpeg-jpg | png |
| 162 | Steganography Studio - BlindHide Algorithm | BlindHide | Windows | Windows | v1.0.2 | png-jpg-bmp | png-bmp |
| 163 | Steganography Studio - SLSB Algorithm | SLSB | Windows | Windows | v1.0.2 | png-jpg-bmp | png-bmp |
| 164 | Steganography Studio - FilterFirst Algorithm | FilterFirst | Windows | Windows | v1.0.2 | png-jpg-bmp | png-bmp |
| 169 | F5_original | F5 | Java | Linux | Downloaded: Oct 27 2022 | jpg-gif | jpg |
| 172 | StegArmory LSB | LSB | Python | Linux | Commit Oct 26 2021 | png | png |
| 178 | SteggoLeggo | LSB | C | Linux | Commit Oct 5 2015 | bmp | bmp |
| 190 | rsteg | LSB | Rust | Linux | Commit Jun 19 2017 | png-jpeg-bmp-jpg | png-jpeg-bmp-jpg |
| 191 | Shusssh! | Unknown | Windows | Windows | - | png-jpeg-gif-bmp-jpg | png-jpeg-gif-bmp-jpg |
| 198 | Free File Camouflage | Unknown | Windows | Windows | v1.25 | jpg | jpg |
| 199 | Steganography Studio - HideSeek Algorithm | HideSeek | Windows | Windows | v1.0.2 | png-jpg-bmp | png-bmp |
| 211 | SteganoG | Unknown | Windows | Windows | - | bmp | bmp |

### What this table says that the abstract does not

**Twenty-seven of the fifty-one are Windows tools**, and twenty-two of those are
Windows binaries rather than portable code. That is the single most important
fact for anyone hoping to reproduce this set: slightly over half of it needs
Windows or Wine, and the REVEAL authors automated one of them, Hide'N'Send, with
Microsoft Power Automate because it has no command line at all.

**The implementations are scattered across eleven languages**: Python, Go, C,
C++, C#, Java, JavaScript, Rust, Python 2, plus the Windows binaries and one
tool that is an online service. There is no single build system that gets you
this set.

**Only nine require a key.** Most embed without a passphrase, which matters for
any harness driving them: the majority need no secret management at all.

**One is flagged by the authors as especially difficult to find**, which is a
polite way of saying the download link had rotted. A snapshot frozen in July
2023 will rot further, and they say so themselves: "REVEAL will be highly
sensitive to software versioning."

### Why this is an opportunity rather than a checklist

The authors excluded academic content-adaptive schemes on purpose, and
stegobench already drives seven of them through `conseal`. So the two sets are
complementary rather than competing: REVEAL holds the real end-user tools,
stegobench holds the adaptive research schemes, and **nobody currently holds
both**.

A maintained, versioned, CI-verified collection of these tools is a genuine
contribution precisely because REVEAL's is a frozen snapshot that the authors
expect to age. The blocker is not engineering. It is redistribution: most of
these are freeware or shareware whose terms do not permit us to ship their
binaries, so a lawful design fetches them at first use under the user's own
acceptance of upstream terms, exactly as the on-demand tier already works.

## What it deliberately leaves open

The authors exclude academic content-adaptive schemes on purpose:

> "we exclude these schemes from our dataset because we believe this comprises a
> much smaller part of steganography in the wild"

They also cover only png, jpg, bmp and gif, hold no faces, logos or number
plates, draw from one country and one season, and freeze their tool snapshot at
July 2023. They say so themselves: "REVEAL will be highly sensitive to software
versioning."

So the gap a new corpus can honestly claim is narrow and specific: **real tools
and adaptive schemes together, at camera diversity, across both spatial and JPEG
domains, as separate arms rather than a blend.** REVEAL holds one half of that
and states plainly that it is not attempting the other.

## Honest positioning

REVEAL is the serious prior art. A reviewer will know it. Any claim this project
makes should be stated as an extension of REVEAL rather than a replacement for
it, because on the axis they chose they are ahead and will stay ahead.

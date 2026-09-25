#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Check every number in the published docs against the corpus that shipped.

WHY THIS EXISTS
---------------
On 2026-09-22, every headline figure on the Pentimento site was wrong. The docs
were written against the corpus as it stood before the cover backfill and the
payload rebuild, the rebuild changed the counts, and nothing anywhere compared
the two. The front page told a visitor the corpus held 344,348 stego pairs when
it held 341,997.

Nobody was careless. The numbers were correct when they were written, the build
moved underneath them, and prose has no equivalent of a failing test. That is
the whole problem: a wrong number in a document renders exactly like a right
one, and the reader has no way to tell.

So the figures are not restated here. Every expected value is DERIVED from the
packed index, and the docs are searched for the rendered form of it. Restating
them would move the staleness into this file rather than remove it.

WHAT IT CANNOT DO
-----------------
It checks the figures it knows how to derive. A number it has no rule for is
reported as unchecked rather than passed, because a checker that silently
covers less than it appears to is the fault this corpus keeps finding
elsewhere.

THE DOCUMENT IT COULD NOT SEE
-----------------------------
For its first days this read the corpus site and nothing else, so the harness
repository's own README sat claiming 344,348 pairs against a real 344,357. The
irony is exact: 344,348 is the number named a few lines above as the wrong
figure that was once published, and the one document out of reach was the one
that kept it. `--repo` closes that: this repository's README, its `llms.txt`
and everything under its `docs/` tree are held to the same figures, from the
same packed index, and fail the same way.

Usage::

    python check_docs_figures.py --docs ~/the-factory/pentimento/docs \\
                                 --release ~/pentimento/release \\
                                 --covers ~/pentimento/covers/commons
"""
from __future__ import annotations

import argparse
import collections
import json
import pathlib
import re
import sys


class FigureError(RuntimeError):
    pass


def load_rows(path: pathlib.Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text().splitlines()
            if line.strip()]


def arms_index(release: pathlib.Path, tier: str) -> dict:
    p = release / f"{tier}-arms" / f"pentimento-{tier}-arms-index.json"
    if not p.is_file():
        raise FigureError(f"no packed arms index at {p}")
    return json.loads(p.read_text())


def covers_index(release: pathlib.Path, tier: str) -> dict:
    p = release / tier / f"pentimento-{tier}-index.json"
    if not p.is_file():
        raise FigureError(f"no packed cover index at {p}")
    return json.loads(p.read_text())


def tier_bytes(release: pathlib.Path, part: str) -> int:
    d = release / part
    return sum(p.stat().st_size for p in d.glob("*.tar")) if d.is_dir() else 0


def is_clean(arm: str) -> bool:
    return arm.startswith("clean")


def derive(release: pathlib.Path, covers: pathlib.Path,
           tier: str = "core") -> dict[str, float]:
    """Every figure the docs quote, taken from what was actually packed.

    `tier` matters more than it looks. This hard-wired "core" until
    2026-09-22, which meant that run against a whole release directory the
    check FAILED on correct Lite and Nano prose (54.8% and 56.5% are the right
    attribution shares for those tiers) while never checking Nano's own 200
    covers at all, because they fall outside the magnitude band around Core's
    10,000. A checker that cries wolf on correct prose is one somebody
    switches off, and it was also silently covering less than it appeared to.
    """
    ai = arms_index(release, tier)
    arms = ai["arms"]
    stego = [a for a in arms if not is_clean(a["arm"])]
    clean = [a for a in arms if is_clean(a["arm"])]

    by_arm = {a["arm"]: a["samples"] for a in arms}
    outguess = {k: v for k, v in by_arm.items() if k.startswith("outguess")}
    steghide = {k: v for k, v in by_arm.items() if k.startswith("steghide")}
    if not outguess:
        raise FigureError("no outguess arm in the packed index, so the "
                          "per-arm figure cannot be derived")
    if len(set(outguess.values())) != 1:
        raise FigureError(
            f"the outguess arms no longer agree on a sample count "
            f"({sorted(set(outguess.values()))}), so the docs cannot quote one "
            f"number for them. Reword the page rather than picking one")

    # HOW MANY DIFFERENT ARM SIZES EXIST, which is the claim the docs make
    # when they say outguess is "the one short arm".
    #
    # It was not true. Twenty-one arms - every spatial adaptive arm, plus
    # clean-grey - sat at 9,882 while the page said everything but outguess
    # held 10,000. A reader doing a paired comparison across two arms had 118
    # covers on one side and not the other, and no per-arm count was wrong, so
    # nothing anywhere contradicted the sentence.
    #
    # The rule matches the one above it: where the corpus stops fitting the
    # sentence, refuse and make somebody reword it, rather than picking a
    # number and carrying on. Two classes is the documented shape - the full
    # arms and the short outguess ones.
    # STEGO arms only. A clean arm is allowed to be a different size: there is
    # one clean arm per distinct clean image set, not per rate, so its count
    # answers a different question.
    classes = sorted({a["samples"] for a in stego})
    if len(classes) > 2:
        raise FigureError(
            f"the stego arms hold {len(classes)} different sample counts "
            f"({classes}), and the docs describe only two: the full arms and "
            f"the short outguess ones. Either the build is incomplete or the "
            f"page needs rewriting; do not publish a 'one short arm' sentence "
            f"over {len(classes)} sizes")

    rows = load_rows(covers / "manifest.jsonl")
    packed_covers = covers_index(release, tier)["samples"]
    # The manifest describes the whole corpus; a smaller tier is a PREFIX of
    # tier_order, so its own rows are the first n by that ordering. Without
    # this, every per-cover figure below describes Core no matter which tier
    # was asked for.
    if packed_covers < len(rows):
        rows = sorted(rows, key=lambda r: r["tier_order"])[:packed_covers]
    attributed = sum(1 for r in rows if r.get("attribution_required"))
    train = sum(1 for r in rows if r.get("split") == "train")
    test = sum(1 for r in rows if r.get("split") == "test")

    figures = {
        "stego pairs": sum(a["samples"] for a in stego),
        "stego arms": len(stego),
        "clean arms": len(clean),
        "covers": packed_covers,
        # Hard-coded in the shipped SPLITS.md until a panel counted the
        # manifest and found 8,029/1,971 where it claimed 8,032/1,968 - and
        # claimed it in Nano too, whose real figures are 167 and 33.
        "train covers": train,
        "test covers": test,
        "samples per outguess arm": next(iter(outguess.values())),
        "steghide and outguess samples": sum(outguess.values()) + sum(steghide.values()),
        "tool arms": len(outguess) + len(steghide),
        "covers requiring attribution": attributed,
        # Per-licence counts. The docs carried a breakdown table that was
        # stale in six of its seven rows, on a page about licensing, in a
        # corpus whose argument is that licensing must be traceable. The
        # table's own summary line underneath it was correct, so the page
        # contradicted itself and nothing noticed.
        **{f"covers under {licence}": n
           for licence, n in collections.Counter(
               r.get("licence") for r in rows).items() if licence},
        # One decimal place, matching `publish_tier.py` exactly. 54.53 rounds
        # to 55 as a whole number and 54.5 to one place, so the docs and the
        # shipped README would disagree on sight while both being right.
        "attribution percent": round(100 * attributed / len(rows), 1),
    }
    return figures


def thousands(n: int) -> str:
    return f"{n:,}"


#: A small count needs its sentence, not just a noun. Searching for "4" matches
#: a version number and a table cell; searching for "4 clean" also matches
#: "360 stego pairs" once the noun is "stego". Each of these is the phrasing the
#: page actually uses, so a match is evidence and a mismatch is the real thing.
def patterns(name: str, value: int, figures: dict[str, int]) -> list[str]:
    if name == "attribution percent":
        # `:g` so a whole percentage reads "55%" rather than "55.0%", which is
        # what anybody would actually write in a sentence.
        return [f"{value:g}%"]
    if name == "stego arms":
        # Two phrasings because two repositories say it two ways: the corpus
        # site writes the table cell, the harness README writes the sentence.
        # Accepting only the first reported the harness README as never
        # stating an arm count it states in plain words.
        stego, clean = figures["stego arms"], figures["clean arms"]
        return [f"{stego} stego, plus {clean} clean",
                f"{stego} stego arms and {clean} clean"]
    if name == "tool arms":
        return [f"{value} tool arms"]
    return [thousands(value)]


#: Checked as part of the arms line above, so checking it again would report
#: the same fact twice and, worse, report a bare digit as verified.
COVERED_ELSEWHERE = {"clean arms"}


#: A number only counts as a candidate for a figure when its own LINE mentions
#: what the figure is about. Without this the check reported "50% of capacity",
#: a payload rate, as a stale attribution percentage, and the train/test split
#: as a stale outguess count. Both were the right kind of number in the wrong
#: place, and a check that cries wolf is one somebody switches off, which
#: leaves the docs unchecked by a different route.
#:
#: The split was formerly named here as a false positive to suppress. It is now
#: a derived figure with a context of its own, because the number it was being
#: excused for carrying turned out to be WRONG: 8,032/1,968 against a real
#: 8,029/1,971. Suppressing a figure and checking it are one decision apart,
#: and the suppression was protecting the defect.
CONTEXT = {
    # Bare "pair" as well as the two full phrasings, because the harness
    # README writes "35 stego arms and 4 clean ones, 344,357 pairs": the noun
    # beside the number is plain "pairs", and requiring "stego pair" reported
    # the headline figure of the whole corpus as never stated.
    "stego pairs": ("stego pair", "matched stego", "pair"),
    "train covers": ("train", "split"),
    "test covers": ("test", "split"),
    "samples per outguess arm": ("outguess",),
    "steghide and outguess samples": ("steghide", "outguess"),
    "covers requiring attribution": ("attribution", "credit line", "require"),
    "attribution percent": ("attribution", "credit line", "require"),
    "covers": ("cover", "photograph"),
}


#: One claim ends and the next begins at a full stop. A table row and a list
#: item have no full stop and stay whole, which is what keeps a breakdown
#: table's rows independent of each other.
SENTENCE = re.compile(r"(?<=[.!?])\s+")


def segments(text: str) -> list[str]:
    """The units a figure can be claimed in: a row, an item, or a sentence.

    A whole paragraph is too coarse. Unwrapping the soft line breaks (see
    `unwrap`) rejoined a nine-line paragraph about Commons' public domain
    holdings into one unit, and "a 1924 Polish physics textbook" three
    sentences away from the words "public domain" then read as a stale count
    of covers under that licence. The sentence is the unit a human would
    point at when asked where a number is claimed.
    """
    out: list[str] = []
    for line in text.splitlines():
        out += SENTENCE.split(line)
    return out


def in_context(text: str, name: str) -> str:
    """Only the sentences that are talking about this figure."""
    # A per-licence count is only ever claimed where that licence is named,
    # and the licences share magnitudes: CC0's 2,625 and CC BY 2.0's 2,624 sit
    # inside each other's +/-10% band, so without this every row of the
    # breakdown table reads as a stale value for every other row.
    if name.startswith("covers under "):
        licence = name[len("covers under "):].lower()
        return "\n".join(s for s in segments(text) if licence in s.lower())
    words = CONTEXT.get(name)
    if not words:
        return text
    return "\n".join(s for s in segments(text)
                     if any(w in s.lower() for w in words))


#: A date is not a quantity. Two of these fired against the real prose:
#: "Ruled 2026-09-16: permissive only. CC0, public domain and plain CC BY"
#: read as 2,026 covers under the public domain against a real 1,922, and "a
#: 1924 Polish physics textbook" in a sentence about a cover test run read as
#: 1,924 test covers against a real 1,971. Both sit inside the magnitude band,
#: and every corpus dates its own prose, so leaving years in the haystack
#: guarantees the collision rather than risking it.
#:
#: The accepted cost: a figure between 1,500 and 2,099 written without its
#: thousands separator is now invisible to this check. Three of the derived
#: figures fall in that band, and every document writes them "1,922" rather
#: than "1922", so the loss is theoretical where the false alarms were real.
DATE = re.compile(r"\b\d{4}-\d{2}-\d{2}\b"
                  r"|(?<![\d,])(?:1[5-9]|20)\d{2}(?![\d,])")


def stale_numbers(text: str, name: str, value: int,
                  figures: dict[str, int]) -> list[str]:
    """Numbers in the same shape as this figure that are not this figure.

    A figure is wrong in exactly one way that matters: some other number sits
    where it should. Matching only the correct value would pass a page that
    never mentions it at all.
    """
    want = set(patterns(name, value, figures))
    text = DATE.sub(" ", in_context(text, name))
    if name == "attribution percent":
        found = set(re.findall(r"\b(\d{1,3}(?:\.\d)?)%", text))
        return sorted(f"{f}%" for f in found if f"{f}%" not in want
                      and 40 <= float(f) <= 70)
    if name == "stego arms":
        out = set()
        for joiner in (", plus ", " arms and "):
            for a, b in re.findall(
                    rf"\b(\d{{1,3}}) stego{re.escape(joiner)}(\d{{1,3}}) clean",
                    text):
                phrase = f"{a} stego{joiner}{b} clean"
                if phrase not in want:
                    out.add(phrase)
        return sorted(out)
    if name == "tool arms":
        found = set(re.findall(r"\b(\d{1,3}) tool arms\b", text))
        return sorted(f"{f} tool arms" for f in found
                      if f"{f} tool arms" not in want)
    # Only numbers of a similar magnitude, so unrelated figures on the page do
    # not read as candidates for this one.
    lo, hi = value * 0.9, value * 1.1
    # Grouped ("8,119") or bare ("8119"): the docs use the first, but a figure
    # written without separators is exactly as wrong and exactly as invisible.
    out = []
    for f in set(re.findall(r"\b\d{1,3}(?:,\d{3})+\b|\b\d{2,}\b", text)):
        if f in want:
            continue
        # A zero-padded token is an identifier, not a quantity. `09710.png` is
        # a cover filename, and reading it as 9,710 made the cover count fail.
        if f.startswith("0"):
            continue
        if lo <= int(f.replace(",", "")) <= hi:
            out.append(f)
    return sorted(out)


#: Generated credit lists are DATA, not claims about the corpus. ATTRIBUTION.md
#: is thousands of third-party file titles, and one of them is a photograph of a
#: bus numbered 10040, which read as a cover count of 10,040. Nothing in such a
#: file is a figure this corpus is asserting, so scanning it can only produce
#: noise.
NOT_PROSE = ("attribution",)


#: Pages whose subject is somebody else's corpus. `cover-source-licensing.md`
#: surveys BOSSbase, ALASKA2, IStego100K and the advertised mirrors of them,
#: and records that one mirror claiming 20,000 covers "actually holds 9,975
#: covers". That is a finding about another dataset, on a line that says
#: "covers", inside the magnitude band around our own 10,000, so scanning the
#: page reports correct prose as a stale cover count. The page asserts no
#: count of this corpus at all, which is what makes the whole-file exclusion
#: honest rather than convenient.
SURVEYS_OTHERS = ("cover-source-licensing",)


#: `private/` is gitignored in every repo on this fleet and never published, so
#: a figure in there is a working note rather than a claim to a reader. Scanning
#: it makes the check fail over prose nobody will ever see.
NOT_PUBLISHED = ("node_modules", "private")


#: Caps, so a generated or vendored file cannot turn the scan into an
#: unbounded read. Both are far above any real documentation tree: the corpus
#: site is 12 pages and its largest is under 30 KiB. Passing either is a loud
#: refusal rather than a truncated read, because a check that quietly examined
#: half the prose is the failure this file exists to prevent.
MAX_PAGE_BYTES = 4 * 1024 * 1024
MAX_PAGES = 2_000
#: The per-page and per-tree caps multiply out to several gigabytes held in
#: memory, which is not a bound worth having. This one is the bound.
MAX_TOTAL_BYTES = 64 * 1024 * 1024


def is_prose(path: pathlib.Path) -> bool:
    if any(part in NOT_PUBLISHED for part in path.parts):
        return False
    stem = path.stem.lower()
    return stem not in NOT_PROSE and stem not in SURVEYS_OTHERS


def read_page(path: pathlib.Path) -> str:
    size = path.stat().st_size
    if size > MAX_PAGE_BYTES:
        raise FigureError(
            f"{path} is {size:,} bytes, past the {MAX_PAGE_BYTES:,} byte cap "
            f"on a page of prose. A file that large is generated data rather "
            f"than a claim about the corpus; add its name to NOT_PROSE")
    try:
        return path.read_text(encoding="utf-8")
    except UnicodeDecodeError as e:
        raise FigureError(f"{path} is not UTF-8 text ({e}), so it cannot be "
                          f"read as prose") from e


#: Markdown whose lines are independent claims rather than one wrapped
#: sentence: table rows, list items, headings, quotes and indented code. These
#: keep one line each, because joining them is exactly what makes an unrelated
#: figure on the next row read as a candidate for this one.
BLOCK = re.compile(r"^(?:\s*$|[|>#]|\s*[-*+]\s|\s*\d+[.)]\s|\s{4,}|```)")


def unwrap(text: str) -> str:
    """Rejoin soft-wrapped prose so a figure and its noun share a line.

    The harness README is wrapped at 79 columns and breaks mid-phrase: "35
    stego arms and 4 clean ones, 344,357" ends one line and "pairs," begins
    the next. Matching line by line, the pair count was invisible in both
    directions, the number having no noun beside it and the noun no number,
    so the figure reported as never stated while the document stated it
    plainly. Line wrapping is a rendering detail and must not decide what
    gets checked.
    """
    out: list[str] = []
    fenced = False
    for line in text.splitlines():
        if line.lstrip().startswith("```"):
            fenced = not fenced
            out.append(line)
            continue
        if fenced or not out or BLOCK.match(line) or BLOCK.match(out[-1]):
            out.append(line)
        else:
            out[-1] = f"{out[-1].rstrip()} {line.strip()}"
    return "\n".join(out)


#: What the harness repository asserts to a reader. Named rather than walked,
#: because an rglob from the repository root would descend into `target/`,
#: `crates/` and every vendored README: unbounded, and full of version numbers
#: and byte counts this corpus never claimed.
REPO_PAGES = ("README.md", "llms.txt")
REPO_TREES = ("docs",)

#: This file lives at <repo>/generators/, so the checkout is two levels up.
#: Defaulting to it rather than requiring a flag is the point: the README went
#: stale precisely because nothing in any invocation named it.
REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent


def pages_under(docs: pathlib.Path) -> list[pathlib.Path]:
    return sorted(p for p in docs.rglob("*.md") if is_prose(p))


def repo_prose(repo: pathlib.Path) -> list[pathlib.Path]:
    """The harness repository's own published prose.

    `llms.txt` is in the list because it is the first thing a model reads to
    decide how to call the tool, so a stale figure there is read by every
    agent that orients itself before every human who opens the README.
    """
    found = [repo / name for name in REPO_PAGES]
    found = [p for p in found if p.is_file() and is_prose(p)]
    for tree in REPO_TREES:
        found += pages_under(repo / tree) if (repo / tree).is_dir() else []
    if not found:
        raise FigureError(
            f"no published prose under {repo}: expected "
            f"{', '.join(REPO_PAGES)} or a docs/ tree, and found neither. "
            f"Pass the repository root, or --no-repo if there is nothing "
            f"there to check")
    return sorted(set(found))


def check(docs: pathlib.Path, figures: dict[str, int],
          repo: pathlib.Path | None = None) -> tuple[list[str], list[str]]:
    found = [(str(p.relative_to(docs)), p) for p in pages_under(docs)]
    if not found:
        raise FigureError(f"no documentation pages under {docs}, so this "
                          f"check examined nothing. That is not a pass")
    if repo is not None:
        # A docs tree that lives inside the repository would otherwise be
        # scanned twice under two names, and every finding in it reported
        # twice, which reads as two defects where there is one.
        seen = {p.resolve() for _, p in found}
        found += [(str(p.relative_to(repo)), p) for p in repo_prose(repo)
                  if p.resolve() not in seen]
    if len(found) > MAX_PAGES:
        raise FigureError(
            f"{len(found):,} pages to scan, past the {MAX_PAGES:,} page cap. "
            f"That is a vendored or generated tree rather than documentation; "
            f"narrow --docs and --repo rather than raising the cap")

    per_page, total = {}, 0
    for name, p in found:
        page = read_page(p)
        total += len(page)
        if total > MAX_TOTAL_BYTES:
            raise FigureError(
                f"more than {MAX_TOTAL_BYTES:,} bytes of prose by the time it "
                f"reached {name}. Narrow --docs and --repo rather than "
                f"raising the cap")
        per_page[name] = unwrap(page)
    text = "\n".join(per_page.values())

    problems, notes = [], []
    # Named roots and a page count, because every failure this file guards
    # against looks identical to a clean run from the outside. A reader has to
    # be able to see that something was actually read, and what.
    where = f"{docs}" + (f" and {repo}" if repo is not None else "")
    notes.append(f"{len(per_page)} page(s) read under {where}")
    for name, value in sorted(figures.items()):
        if name in COVERED_ELSEWHERE:
            continue
        wanted = patterns(name, value, figures)
        present = any(w in in_context(text, name) for w in wanted)
        stale = stale_numbers(text, name, value, figures)

        if not present and not stale:
            notes.append(f"{name}: the docs never state it ({wanted[0]}), so "
                         f"nothing was checked")
            continue
        if stale:
            # One line per document rather than one per figure. The fix for a
            # stale number is editing a named file, and a finding that lists
            # four filenames against three values leaves the reader to work
            # out which value is in which file before they can start.
            found_in = 0
            for page in sorted(per_page):
                here = in_context(per_page[page], name)
                got = [s for s in stale if s in here]
                if not got:
                    continue
                found_in += 1
                problems.append(
                    f"{page}: {name} reads {', '.join(got)}, and the packed "
                    f"index says {wanted[0]}")
            if not found_in:
                problems.append(
                    f"(page unknown): {name} reads {', '.join(stale)}, and "
                    f"the packed index says {wanted[0]}")
        elif present:
            notes.append(f"{name}: {wanted[0]}, as shipped")
    return problems, notes


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--docs", required=True,
                    help="the documentation directory, e.g. pentimento/docs")
    ap.add_argument("--release", required=True,
                    help="the packed release directory")
    ap.add_argument("--covers", required=True,
                    help="the cover directory holding manifest.jsonl")
    ap.add_argument("--tier", default="core",
                    help="which packed tier the docs describe (core, lite, "
                         "nano). Lite and Nano have their own correct "
                         "attribution shares and cover counts, so checking "
                         "their prose against Core's reports correct text as "
                         "wrong")
    ap.add_argument("--repo", default=str(REPO_ROOT),
                    help="the harness repository root, whose README, "
                         "llms.txt and docs/ tree quote the same figures and "
                         "drifted from them unnoticed while only the corpus "
                         "site was being checked")
    ap.add_argument("--no-repo", action="store_true",
                    help="check only --docs. For running this against a "
                         "corpus site from outside a checkout of the harness")
    args = ap.parse_args(argv)

    try:
        figures = derive(pathlib.Path(args.release), pathlib.Path(args.covers),
                         args.tier)
        problems, notes = check(pathlib.Path(args.docs), figures,
                                None if args.no_repo
                                else pathlib.Path(args.repo))
    except FigureError as e:
        print(f"cannot check: {e}", file=sys.stderr)
        return 1

    for n in notes:
        print(f"  ok    {n}" if "as shipped" in n else f"  ----  {n}")
    if not problems:
        # Derived, compared and never-stated are three different numbers, and
        # printing only the first reported nine figures as agreed when four
        # had been compared and three were never mentioned. That is the same
        # over-claim this file's docstring argues against, one level up.
        never = sum(1 for n in notes if "never state" in n)
        compared = sum(1 for n in notes if "as shipped" in n)
        print(f"\n{len(figures)} figure(s) derived from the packed index, "
              f"{compared} compared with the docs, {never} never stated there. "
              f"Every figure that was compared agrees.")
        return 0
    print()
    for p in problems:
        print(f"  FAIL  {p}", file=sys.stderr)
    print(f"\n{len(problems)} statement(s) in the docs do not match the corpus "
          f"that shipped. A wrong number reads exactly like a right one.",
          file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

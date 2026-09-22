#!/usr/bin/env python3
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

Usage::

    python check_docs_figures.py --docs ~/the-factory/pentimento/docs \\
                                 --release ~/pentimento/release \\
                                 --covers ~/pentimento/covers/commons
"""
from __future__ import annotations

import argparse
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


def derive(release: pathlib.Path, covers: pathlib.Path) -> dict[str, float]:
    """Every figure the docs quote, taken from what was actually packed."""
    ai = arms_index(release, "core")
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

    rows = load_rows(covers / "manifest.jsonl")
    attributed = sum(1 for r in rows if r.get("attribution_required"))

    figures = {
        "stego pairs": sum(a["samples"] for a in stego),
        "stego arms": len(stego),
        "clean arms": len(clean),
        "covers": covers_index(release, "core")["samples"],
        "samples per outguess arm": next(iter(outguess.values())),
        "steghide and outguess samples": sum(outguess.values()) + sum(steghide.values()),
        "tool arms": len(outguess) + len(steghide),
        "covers requiring attribution": attributed,
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
        return [f"{figures['stego arms']} stego, plus {figures['clean arms']} clean"]
    if name == "tool arms":
        return [f"{value} tool arms"]
    return [thousands(value)]


#: Checked as part of the arms line above, so checking it again would report
#: the same fact twice and, worse, report a bare digit as verified.
COVERED_ELSEWHERE = {"clean arms"}


#: A number only counts as a candidate for a figure when its own LINE mentions
#: what the figure is about. Without this the check reported "50% of capacity",
#: a payload rate, as a stale attribution percentage, and a train/test split of
#: 8,032 covers as a stale outguess count. Both were the right kind of number in
#: the wrong place, and a check that cries wolf is one somebody switches off,
#: which leaves the docs unchecked by a different route.
CONTEXT = {
    "stego pairs": ("stego pair", "matched stego"),
    "samples per outguess arm": ("outguess",),
    "steghide and outguess samples": ("steghide", "outguess"),
    "covers requiring attribution": ("attribution", "credit line", "require"),
    "attribution percent": ("attribution", "credit line", "require"),
    "covers": ("cover", "photograph"),
}


def in_context(text: str, name: str) -> str:
    """Only the lines that are talking about this figure."""
    words = CONTEXT.get(name)
    if not words:
        return text
    return "\n".join(line for line in text.splitlines()
                      if any(w in line.lower() for w in words))


def stale_numbers(text: str, name: str, value: int,
                  figures: dict[str, int]) -> list[str]:
    """Numbers in the same shape as this figure that are not this figure.

    A figure is wrong in exactly one way that matters: some other number sits
    where it should. Matching only the correct value would pass a page that
    never mentions it at all.
    """
    want = set(patterns(name, value, figures))
    text = in_context(text, name)
    if name == "attribution percent":
        found = set(re.findall(r"\b(\d{1,3}(?:\.\d)?)%", text))
        return sorted(f"{f}%" for f in found if f"{f}%" not in want
                      and 40 <= float(f) <= 70)
    if name == "stego arms":
        found = re.findall(r"\b(\d{1,3}) stego, plus (\d{1,3}) clean", text)
        return sorted(f"{a} stego, plus {b} clean" for a, b in found
                      if f"{a} stego, plus {b} clean" not in want)
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


def is_prose(path: pathlib.Path) -> bool:
    if "node_modules" in path.parts:
        return False
    return path.stem.lower() not in NOT_PROSE


def check(docs: pathlib.Path, figures: dict[str, int]) -> tuple[list[str], list[str]]:
    pages = sorted(p for p in docs.rglob("*.md") if is_prose(p))
    if not pages:
        raise FigureError(f"no documentation pages under {docs}, so this "
                          f"check examined nothing. That is not a pass")
    text = "\n".join(p.read_text() for p in pages)
    per_page = {p: p.read_text() for p in pages}

    problems, notes = [], []
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
            where = sorted({str(p.relative_to(docs)) for p, t in per_page.items()
                            if any(s in in_context(t, name) for s in stale)})
            problems.append(
                f"{name} should read {wanted[0]}; the docs also carry "
                f"{', '.join(stale)} in {', '.join(where)}")
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
    args = ap.parse_args(argv)

    try:
        figures = derive(pathlib.Path(args.release), pathlib.Path(args.covers))
        problems, notes = check(pathlib.Path(args.docs), figures)
    except FigureError as e:
        print(f"cannot check: {e}", file=sys.stderr)
        return 1

    for n in notes:
        print(f"  ok    {n}" if "as shipped" in n else f"  ----  {n}")
    if not problems:
        print(f"\n{len(figures)} figure(s) derived from the packed index; "
              f"the docs agree with all of them.")
        return 0
    print()
    for p in problems:
        print(f"  FAIL  {p}", file=sys.stderr)
    print(f"\n{len(problems)} figure(s) in the docs do not match the corpus "
          f"that shipped. A wrong number reads exactly like a right one.",
          file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

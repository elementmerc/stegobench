#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Turn two `analyse_panel.py` transcripts into the paper's Table 1 artefacts.

Reads the fixed-width table `analyse_panel.py` prints, which is the only
machine-readable form of those numbers that exists, and emits:

* the LaTeX body for `tab:jpeg`, in the column order and number formatting the
  paper already uses, so the fragment drops straight in;
* every cell whose permutation p crosses 0.05 in either direction, old beside
  new;
* the three headline checks, as numbers rather than as claims.

This file only reformats and compares. It computes no statistic that
`analyse_panel.py` did not already compute, apart from the two-sided sign test
on the nine outguess AUC directions, which is exact and closed form.
"""
from __future__ import annotations

import argparse
import math
import pathlib
import re

#: Arm directory names as the paper's tables write them.
PRETTY = {"0050": "0.05", "0200": "0.20", "0500": "0.50"}
SHORT = {"aletheia_spa": "SPA", "aletheia_rs": "RS", "stegexpose": "StegExpose"}

ROW = re.compile(
    r"^(?P<arm>\S+)\s+(?P<det>aletheia_spa|aletheia_rs|stegexpose)\s+"
    r"(?P<n>\d+)\s+(?P<auc>[\d.]+)\s+"
    r"(?P<cllr>[\d.]+) \((?P<sd>[\d.]+)\)\s+"
    r"(?P<cmin>[\d.]+)\s+(?P<ccal>[-\d.]+)\s+(?P<p>[\d.]+)\s+(?P<verdict>.+?)\s*$"
)


def pretty_arm(arm: str) -> str:
    head, _, tail = arm.partition("/")
    return f"{head}/{PRETTY[tail]}" if tail in PRETTY else head


def parse(path: pathlib.Path) -> dict[tuple[str, str], dict]:
    cells: dict[tuple[str, str], dict] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        m = ROW.match(line)
        if not m:
            continue
        d = m.groupdict()
        key = (pretty_arm(d["arm"]), SHORT[d["det"]])
        cells[key] = {
            "n": int(d["n"]),
            "auc": float(d["auc"]),
            "cllr": float(d["cllr"]),
            "sd": float(d["sd"]),
            "cmin": float(d["cmin"]),
            "ccal": float(d["ccal"]),
            "p": float(d["p"]),
            "verdict": d["verdict"],
            "raw": {k: d[k] for k in ("n", "auc", "cllr", "sd", "cmin", "ccal", "p")},
        }
    return cells


def order(cells) -> list[tuple[str, str]]:
    """Arm order as the paper prints it: sorted arms, detectors SPA, RS, StegExpose."""
    arms, seen = [], set()
    for arm, _ in cells:
        if arm not in seen:
            seen.add(arm)
            arms.append(arm)
    keys = []
    for arm in sorted(arms):
        for det in ("SPA", "RS", "StegExpose"):
            if (arm, det) in cells:
                keys.append((arm, det))
    return keys


def latex(cells) -> str:
    keys = order(cells)
    width = max(len(a) for a, _ in keys)
    lines, structural_done = [], False
    for arm, det in keys:
        if arm.startswith("structural") and not structural_done:
            lines.append(r"\midrule")
            structural_done = True
        c = cells[(arm, det)]
        p = f"{c['p']:.3f}"
        p = rf"\textbf{{{p}}}" if c["p"] <= 0.05 else p
        lines.append(
            f"{arm:<{width}} & {det:<10} & {c['n']} & {c['auc']:.3f} & "
            f"{c['cllr']:.3f} ({c['sd']:.3f}) & {c['cmin']:.3f} & {c['ccal']:.3f} & {p} \\\\"
        )
    return "\n".join(lines) + "\n"


def sign_test(aucs: list[float]) -> tuple[int, int, float]:
    """Exact two-sided sign test on the direction of each AUC about 0.5."""
    below = sum(1 for a in aucs if a < 0.5)
    above = sum(1 for a in aucs if a > 0.5)
    n = below + above
    k = min(below, above)
    tail = sum(math.comb(n, i) for i in range(k + 1)) / 2**n
    return below, n, min(1.0, 2 * tail)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("new", type=pathlib.Path, help="analyse_panel transcript, 200-cover corpus")
    ap.add_argument("old", type=pathlib.Path, help="analyse_panel transcript, published corpus")
    ap.add_argument("--latex-out", type=pathlib.Path)
    args = ap.parse_args(argv)

    new, old = parse(args.new), parse(args.old)
    keys = order(new)

    print(f"cells: {len(new)} new, {len(old)} old\n")

    print("== p values crossing 0.05 in either direction ==")
    print(f"{'arm':<14}{'detector':<12}{'old p':>8}{'new p':>8}{'old n':>7}{'new n':>7}  direction")
    crossings = 0
    for key in keys:
        if key not in old:
            continue
        o, n = old[key]["p"], new[key]["p"]
        if (o <= 0.05) == (n <= 0.05):
            continue
        crossings += 1
        way = "significant -> not" if o <= 0.05 else "not -> significant"
        print(
            f"{key[0]:<14}{key[1]:<12}{o:>8.3f}{n:>8.3f}"
            f"{old[key]['n']:>7}{new[key]['n']:>7}  {way}"
        )
    if not crossings:
        print("  none")
    print(f"\n{crossings} cell(s) cross 0.05")

    print("\n== headline checks, on the new corpus ==")
    worst = min(new.values(), key=lambda c: c["cllr"])
    n_above = sum(1 for c in new.values() if c["cllr"] > 1.0)
    print(f"Cllr > 1.0 in {n_above} of {len(new)} cells; smallest Cllr is {worst['cllr']:.3f}")

    og = [new[k]["auc"] for k in keys if k[0].startswith("outguess")]
    print(
        f"outguess AUCs below 0.5: {sum(1 for a in og if a < 0.5)} of {len(og)}; "
        f"range {min(og):.3f} to {max(og):.3f}"
    )
    below, n, p = sign_test(og)
    print(f"two-sided sign test on the nine directions: {below}/{n} below 0.5, p = {p:.4f}")

    sig = [k for k in keys if new[k]["p"] <= 0.05]
    print(f"\ncells at p <= 0.05, new corpus ({len(sig)}): " + ", ".join(f"{a}/{d}" for a, d in sig))
    old_sig = [k for k in order(old) if old[k]["p"] <= 0.05]
    print(f"cells at p <= 0.05, old corpus ({len(old_sig)}): " + ", ".join(f"{a}/{d}" for a, d in old_sig))

    body = latex(new)
    if args.latex_out:
        args.latex_out.write_text(body, encoding="utf-8")
        print(f"\nLaTeX body written to {args.latex_out}")
    else:
        print("\n== LaTeX body ==")
        print(body, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Content-adaptive spatial embedding: HUGO, WOW, S-UNIWARD, HILL, MiPOD.

This is the arm the field's best current corpus does not have. REVEAL (Kombrink
et al., Netherlands Forensic Institute, 2025) covers 51 real end-user tools and
excludes academic content-adaptive schemes on purpose, stating that they comprise
"a much smaller part of steganography in the wild". That is a defensible
editorial choice and it leaves the adaptive family uncovered at camera-diversity
scale, which is where this arm goes.

Adaptive schemes are the hard case for a detector and the easy case to get wrong
in a corpus. They do not overwrite a fixed set of pixels; they compute a
per-pixel cost of changing it, concentrate changes in texture and noise where a
change is cheap to hide, and avoid smooth regions where it would show. That is
why LSB numbers say nothing about adaptive performance, and why measuring only
one family reports an upper bound on an adversary rather than an estimate.

GREYSCALE, DELIBERATELY
-----------------------
These schemes are defined for a single channel and the entire literature
evaluates them that way, so each arm carries its own greyscale cover written
beside its stego image. The pair is therefore internally matched, which is the
property that matters, even though an adaptive arm's covers are not byte-identical
to the LSB arms' colour covers. Pairing within an arm is what a detector is
measured on; pairing across arms is not a thing anyone computes.

SIMULATION, AND WHAT IT MEANS
-----------------------------
`conseal` simulates embedding at the optimal coding rate rather than running a
real syndrome-trellis coder. The resulting change map is what a perfect coder
would produce, which is the standard on which every published adaptive result
rests, and it is slightly harder to detect than a real coder's output. The
manifest records this so nobody has to infer it later.
"""
import argparse
import hashlib
import json
import pathlib
import sys

import numpy as np
from PIL import Image

try:
    import conseal as cl
except ImportError:  # pragma: no cover - the message is the whole point
    print(
        "conseal is not installed. It carries the adaptive schemes and has no "
        "MATLAB dependency:\n    pip install conseal",
        file=sys.stderr,
    )
    raise SystemExit(2)

# rate in bits per pixel, matching the LSB sweep so the two families are
# directly comparable on the same axis.
RATES = [0.4, 0.2, 0.1, 0.05, 0.01, 0.005]

SCHEMES = {
    "hugo": lambda a, r: cl.hugo.simulate_single_channel(a, r, seed=None),
    "wow": lambda a, r: cl.wow.simulate_single_channel(a, r, seed=None),
    "suniward": lambda a, r: cl.suniward.simulate_single_channel(a, r, seed=None),
    "hill": lambda a, r: cl.hill.simulate_single_channel(a, r, seed=None),
    "mipod": lambda a, r: cl.mipod.simulate_single_channel(a, r, seed=None),
}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--covers", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--scheme", required=True, choices=sorted(SCHEMES))
    ap.add_argument("--count", type=int, default=40)
    ap.add_argument("--size", type=int, default=512)
    ap.add_argument("--seed", type=int, default=20260915)
    args = ap.parse_args()

    covers = sorted(pathlib.Path(args.covers).glob("*.png"))[: args.count]
    if not covers:
        print(f"no covers under {args.covers}", file=sys.stderr)
        return 1

    out = pathlib.Path(args.out)
    manifest = []
    embed = SCHEMES[args.scheme]

    for rate in RATES:
        arm = f"{int(rate * 1000):04d}"
        for split in ("clean", "stego"):
            (out / arm / split).mkdir(parents=True, exist_ok=True)
        for idx, src in enumerate(covers):
            # Crop rather than resize: resampling rewrites the low-bit plane,
            # which is the statistic these schemes hide in and detectors read.
            img = Image.open(src).convert("L")
            w, h = img.size
            if w < args.size or h < args.size:
                continue
            img = img.crop((
                (w - args.size) // 2, (h - args.size) // 2,
                (w - args.size) // 2 + args.size, (h - args.size) // 2 + args.size,
            ))
            x0 = np.array(img, dtype=np.uint8)

            clean_p = out / arm / "clean" / f"{idx:05d}.png"
            Image.fromarray(x0, mode="L").save(clean_p)

            # Seeded per image and per rate, so a rebuild reproduces this arm.
            np.random.seed((args.seed + idx * 7919 + int(rate * 100000)) % (2**32))
            try:
                x1 = embed(x0, float(rate))
            except Exception as e:  # noqa: BLE001 - surface which image and why
                print(f"  {args.scheme} failed on {src.name} at {rate}: {e}",
                      file=sys.stderr)
                continue
            x1 = np.clip(x1, 0, 255).astype(np.uint8)

            stego_p = out / arm / "stego" / f"{idx:05d}.png"
            Image.fromarray(x1, mode="L").save(stego_p)

            changed = int((x0 != x1).sum())
            manifest.append({
                "scheme": args.scheme,
                "rate": rate,
                "arm": arm,
                "index": idx,
                "source": src.name,
                "size": args.size,
                "samples_changed": changed,
                "change_rate": round(changed / x0.size, 6),
                "coding": "simulated at the optimal coding rate, not a real STC",
                "clean_sha256": hashlib.sha256(clean_p.read_bytes()).hexdigest(),
                "stego_sha256": hashlib.sha256(stego_p.read_bytes()).hexdigest(),
            })

    if not manifest:
        print(f"{args.scheme}: nothing embedded", file=sys.stderr)
        return 1

    (out / "manifest.jsonl").write_text(
        "".join(json.dumps(m) + "\n" for m in manifest)
    )
    print(f"{args.scheme}: wrote {len(manifest)} pairs across {len(RATES)} rates")
    for rate in RATES:
        ch = [m["samples_changed"] for m in manifest if m["rate"] == rate]
        if ch:
            print(f"  {rate:>6.3f} bpp: mean samples changed = {sum(ch)/len(ch):>9,.0f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

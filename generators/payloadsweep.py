#!/usr/bin/env python3
"""Detection against payload rate, at a fixed image size.

The size sweep refuted the resize hypothesis: detection got BETTER with image
size, not worse. The explanation that fits is that a fixed bits-per-pixel rate
means a larger image carries more absolute perturbation, so more residual noise
survives the downscale to 448. If that is right then what the subject is
actually responding to is the raised noise floor, and the variable that governs
it is how much of the image was touched.

So this holds the size constant and sweeps the payload rate across the range
that matters operationally. Real hidden messages are small: a 10 kB note in a
512x512 RGB image is about 0.1 bpp. Half a bit per pixel, which is where both
previous runs sat, is an enormous payload that no careful operator would use.

Rates run from 0.5 down to 0.005. The bottom of that range is where classical
steganalysis is known to fail too, so it is the honest place to compare rather
than a trap.
"""
import argparse
import hashlib
import json
import pathlib
import sys

import numpy as np
from PIL import Image

RATES = [0.5, 0.25, 0.1, 0.05, 0.01, 0.005]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--covers", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--count", type=int, default=40)
    ap.add_argument("--size", type=int, default=512)
    ap.add_argument("--seed", type=int, default=20260915)
    ap.add_argument(
        "--method",
        choices=("replace", "match"),
        default="replace",
        help="replace: overwrite the low bit (LSB replacement), the easy case "
        "every classical detector is built for. match: add or subtract one so "
        "the low bit comes out right (LSB matching, plus or minus 1), which "
        "leaves the pair statistics SPA, RS and WS read almost untouched.",
    )
    ap.add_argument(
        "--placement",
        choices=("spread", "sequential"),
        default="spread",
        help="spread: payload scattered over the whole image, as real tools do. "
        "sequential: packed into a prefix, which global detectors dilute away.",
    )
    args = ap.parse_args()

    covers = sorted(pathlib.Path(args.covers).glob("*.png"))[: args.count]
    if not covers:
        print(f"no covers under {args.covers}", file=sys.stderr)
        return 1

    out = pathlib.Path(args.out)
    manifest = []
    for rate in RATES:
        # Directory name carries the rate in per-mille so it stays an integer
        # and sorts correctly; the manifest keeps the real number.
        arm = f"{int(rate * 1000):04d}"
        for split in ("clean", "stego"):
            (out / arm / split).mkdir(parents=True, exist_ok=True)
        for idx, src in enumerate(covers):
            img = Image.open(src).convert("RGB").resize(
                (args.size, args.size), Image.LANCZOS
            )
            arr = np.array(img, dtype=np.uint8)

            clean_p = out / arm / "clean" / f"{idx:05d}.png"
            Image.fromarray(arr).save(clean_p)

            nbits = int(arr.size * rate)
            rng = np.random.default_rng(args.seed + idx)
            bits = rng.integers(0, 2, size=nbits, dtype=np.uint8)
            flat = arr.reshape(-1).copy()

            if args.placement == "spread":
                # Scatter the payload over the whole image, which is what every
                # real tool does and what the classical detectors assume.
                #
                # The first version of this script filled flat[:nbits], a
                # sequential prefix. At 0.25 bpp that puts the entire payload in
                # the first quarter of the image and leaves three quarters
                # pristine. SPA, RS and WS are GLOBAL estimators: they measure
                # the disturbance across the whole LSB plane, so a concentrated
                # payload is diluted by the untouched majority and the estimated
                # rate comes out far below the true one. Measured on 2026-09-15,
                # that alone took Stegcore's verdict from what its calibration
                # predicts down to near zero, which looked like a calibration
                # fault and was an artefact of this line.
                pos = rng.choice(flat.size, size=nbits, replace=False)
            else:
                pos = np.arange(nbits)

            if args.method == "replace":
                flat[pos] = (flat[pos] & 0xFE) | bits
            else:
                # LSB matching. Where the low bit is already right, touch
                # nothing: that is what keeps the sample-pair statistics clean.
                # Otherwise step the value by one in a random direction, which
                # moves the sample between pairs rather than within one, so the
                # structure SPA and RS look for never forms. Clamp at the ends,
                # since 0 cannot go down and 255 cannot go up.
                cur = flat[pos]
                need = cur & 1 != bits
                step = rng.integers(0, 2, size=pos.size, dtype=np.int16) * 2 - 1
                new = cur.astype(np.int16) + np.where(need, step, 0)
                new = np.where(new < 0, 1, new)
                new = np.where(new > 255, 254, new)
                flat[pos] = new.astype(np.uint8)
            stego_p = out / arm / "stego" / f"{idx:05d}.png"
            Image.fromarray(flat.reshape(arr.shape)).save(stego_p)

            changed = int(
                (np.array(Image.open(clean_p)) != np.array(Image.open(stego_p))).sum()
            )
            manifest.append(
                {
                    "rate": rate,
                    "arm": arm,
                    "index": idx,
                    "source": src.name,
                    "size": args.size,
                    "bits": nbits,
                    "method": args.method,
                    "placement": args.placement,
                    "samples_changed": changed,
                    "clean_sha256": hashlib.sha256(clean_p.read_bytes()).hexdigest(),
                    "stego_sha256": hashlib.sha256(stego_p.read_bytes()).hexdigest(),
                }
            )

    (out / "manifest.jsonl").write_text(
        "".join(json.dumps(m) + "\n" for m in manifest)
    )
    print(f"wrote {len(manifest)} pairs across {len(RATES)} payload rates at {args.size}px, placement={args.placement}, method={args.method}")
    for rate in RATES:
        ch = [m["samples_changed"] for m in manifest if m["rate"] == rate]
        approx_bytes = int(RATES and manifest[0]["size"] ** 2 * 3 * rate / 8)
        print(
            f"  {rate:>6.3f} bpp: mean samples changed = {sum(ch)/len(ch):>10,.0f}"
            f"   (~{approx_bytes:,} bytes of payload)"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

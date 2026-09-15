#!/usr/bin/env python3
"""Does StegaShield's 448x448 input resize destroy the signal it looks for?

The hypothesis, from the image metadata (MODEL_INPUT_SIZE=448): every image is
scaled to 448x448 before inference. LSB steganography lives in the lowest bit of
individual pixels, and any resampling averages neighbouring pixels together, so
the payload is gone before the model sees it.

If that is right, detection should appear at or below 448 (where no downscale
happens) and vanish above it. Payload rate is held constant in bits per pixel,
so the only variable across arms is the image size.

Covers are taken from the corpus already on disk and resized first, then
embedded, so the cover and its stego twin differ ONLY in the embedded bits.
That pairing is the whole point: the previous run showed the model's response to
image content is about a thousand times larger than its response to a payload,
so an unpaired comparison would measure content, not detection.
"""
import argparse
import hashlib
import json
import pathlib
import sys

import numpy as np
from PIL import Image

SIZES = [224, 336, 448, 512, 768, 1024]


def lsb_replace(arr: np.ndarray, bits: np.ndarray) -> np.ndarray:
    """Overwrite the low bit of the first `len(bits)` samples, in place order.

    LSB *replacement*, not matching: this is the easy case that every classical
    detector is built for. If the subject cannot see this, it cannot see
    anything harder either.
    """
    flat = arr.reshape(-1).copy()
    n = min(len(bits), flat.size)
    flat[:n] = (flat[:n] & 0xFE) | bits[:n].astype(flat.dtype)
    return flat.reshape(arr.shape)


def payload_bits(n: int, seed: int) -> np.ndarray:
    """Ciphertext-like bits: a real payload is encrypted, so it is uniform."""
    rng = np.random.default_rng(seed)
    return rng.integers(0, 2, size=n, dtype=np.uint8)



def open_preserving_mode(path) -> Image.Image:
    """Open an image as greyscale or RGB, whichever it already is.

    BOSSbase is single-channel greyscale, and forcing it to RGB would replicate
    one plane three times: every payload bit would then be embedded three times
    over, in three perfectly correlated channels. No real embedder does that,
    and it would make the arm easier than reality while looking like a detail of
    file handling. Palette images are promoted to RGB because their pixel values
    are indices, not intensities, and a low bit of an index means nothing.
    """
    img = Image.open(path)
    if img.mode in ("L", "I;16", "I"):
        return img.convert("L")
    return img.convert("RGB")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--covers", required=True, help="directory of source covers")
    ap.add_argument("--out", required=True, help="output root")
    ap.add_argument("--count", type=int, default=40)
    ap.add_argument("--bpp", type=float, default=0.5, help="bits per sample")
    ap.add_argument("--seed", type=int, default=20260915)
    args = ap.parse_args()

    covers = sorted(pathlib.Path(args.covers).glob("*.png"))[: args.count]
    if not covers:
        print(f"no covers under {args.covers}", file=sys.stderr)
        return 1

    out = pathlib.Path(args.out)
    manifest = []
    for size in SIZES:
        for split in ("clean", "stego"):
            (out / str(size) / split).mkdir(parents=True, exist_ok=True)
        for idx, src in enumerate(covers):
            img = open_preserving_mode(src).resize((size, size), Image.LANCZOS)
            arr = np.array(img, dtype=np.uint8)

            # The cover is written from the SAME resized array the stego is made
            # from, so resampling artefacts are identical in both arms and the
            # only difference is the payload.
            clean_p = out / str(size) / "clean" / f"{idx:05d}.png"
            Image.fromarray(arr).save(clean_p)

            nbits = int(arr.size * args.bpp)
            bits = payload_bits(nbits, args.seed + idx)
            stego_p = out / str(size) / "stego" / f"{idx:05d}.png"
            Image.fromarray(lsb_replace(arr, bits)).save(stego_p)

            changed = int((np.array(Image.open(clean_p)) != np.array(Image.open(stego_p))).sum())
            manifest.append(
                {
                    "size": size,
                    "index": idx,
                    "source": src.name,
                    "bpp": args.bpp,
                    "bits": nbits,
                    "samples_changed": changed,
                    "clean_sha256": hashlib.sha256(clean_p.read_bytes()).hexdigest(),
                    "stego_sha256": hashlib.sha256(stego_p.read_bytes()).hexdigest(),
                }
            )

    (out / "manifest.jsonl").write_text(
        "".join(json.dumps(m) + "\n" for m in manifest)
    )
    per_size = {}
    for m in manifest:
        per_size.setdefault(m["size"], []).append(m["samples_changed"])
    print(f"wrote {len(manifest)} pairs across {len(SIZES)} sizes")
    for size, ch in sorted(per_size.items()):
        print(f"  {size:>5}px: mean samples changed per image = {sum(ch)/len(ch):,.0f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""The structural control: data appended after the PNG end-of-image marker.

This is the crudest hiding technique there is. The payload is not woven into the
pixels at all, it is simply stapled to the end of the file after IEND, where any
tool that reads the container rather than the picture will trip over it. Stegcore
has a detector for exactly this, and so does every serious steganalysis tool.

It is in this evaluation as a floor, not as a challenge. A subject that misses
LSB embedding might reasonably be said to be looking at a hard problem. A subject
that also misses bytes bolted onto the end of the file is not doing file
analysis at all, whatever the marketing says about entropy anomaly scoring.

Pairing rule as everywhere else: the clean arm is the same image written the same
way, so the only difference is the appended block.
"""
import argparse
import hashlib
import json
import pathlib
import secrets
import sys

IEND = b"\x49\x45\x4e\x44\xae\x42\x60\x82"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--covers", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--count", type=int, default=40)
    ap.add_argument("--payload-bytes", type=int, default=4096)
    args = ap.parse_args()

    covers = sorted(pathlib.Path(args.covers).glob("*.png"))[: args.count]
    if not covers:
        print(f"no covers under {args.covers}", file=sys.stderr)
        return 1

    out = pathlib.Path(args.out)
    for split in ("clean", "stego"):
        (out / split).mkdir(parents=True, exist_ok=True)

    manifest = []
    for idx, src in enumerate(covers):
        raw = src.read_bytes()
        if not raw.rstrip().endswith(IEND):
            # Refuse to guess at a container we do not recognise rather than
            # append to something that is not a PNG.
            print(f"skipping {src.name}: no IEND at end of file", file=sys.stderr)
            continue
        clean_p = out / "clean" / f"{idx:05d}.png"
        stego_p = out / "stego" / f"{idx:05d}.png"
        # Ciphertext-like, so the appended block is high entropy, which is the
        # thing an entropy-based detector is supposed to notice.
        blob = secrets.token_bytes(args.payload_bytes)
        clean_p.write_bytes(raw)
        stego_p.write_bytes(raw + blob)
        manifest.append(
            {
                "index": idx,
                "source": src.name,
                "appended_bytes": len(blob),
                "clean_sha256": hashlib.sha256(raw).hexdigest(),
                "stego_sha256": hashlib.sha256(raw + blob).hexdigest(),
            }
        )

    (out / "manifest.jsonl").write_text(
        "".join(json.dumps(m) + "\n" for m in manifest)
    , encoding="utf-8")
    print(f"wrote {len(manifest)} pairs, {args.payload_bytes} bytes appended after IEND")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

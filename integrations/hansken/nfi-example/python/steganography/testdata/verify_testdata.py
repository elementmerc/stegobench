#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright (C) 2026 Daniel Iwugo
"""Check the fixtures are what make_testdata.py's docstring says they are.

Run from this directory: python verify_testdata.py

Every figure quoted in that docstring is asserted here. An earlier version
quoted numbers written before the fixtures were generated, and they were
wrong, which is the whole reason this file exists.
"""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))

from trailing_data import find_trailing  # noqa: E402

INPUT = pathlib.Path(__file__).parent / 'input'


def naive_first_eoi(data: bytes) -> int:
    for i in range(len(data) - 1):
        if data[i] == 0xFF and data[i + 1] == 0xD9:
            return i + 2
    raise AssertionError('no EOI at all')


def main() -> int:
    jpeg = (INPUT / 'clean_jpg.raw').read_bytes()
    markers = [i for i in range(len(jpeg) - 1) if jpeg[i] == 0xFF and jpeg[i + 1] == 0xD9]
    naive = naive_first_eoi(jpeg)
    mistaken = len(jpeg) - naive

    checks = [
        ('jpeg length is 8026', len(jpeg) == 8026),
        ('EOI markers at 660 and 8024', markers == [660, 8024]),
        ('naive search stops at 662', naive == 662),
        ('naive would report 7364 bytes', mistaken == 7364),
        ('which is 91.8% of the file', abs(mistaken / len(jpeg) - 0.918) < 0.001),
        ('the segment walk reports nothing trailing', find_trailing(jpeg).length == 0),
        ('clean png has nothing trailing', find_trailing((INPUT / 'clean_png.raw').read_bytes()).length == 0),
        ('appended png carries 3392 bytes of text', (
            find_trailing((INPUT / 'appended_png.raw').read_bytes()).length == 3392
            and find_trailing((INPUT / 'appended_png.raw').read_bytes()).looks_like == 'text'
        )),
        ('appended png splits at the clean png length', (
            find_trailing((INPUT / 'appended_png.raw').read_bytes()).offset
            == (INPUT / 'clean_png.raw').stat().st_size
        )),
    ]

    failed = 0
    for label, ok in checks:
        print(f'{"ok  " if ok else "FAIL"}  {label}')
        failed += not ok
    return 1 if failed else 0


if __name__ == '__main__':
    raise SystemExit(main())

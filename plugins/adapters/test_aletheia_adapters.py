#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests holding the batched Aletheia adapter to the single image one.

Nothing here needs Aletheia, numpy or imageio installed. Both adapters import
them inside `main`, so a fake module is put in `sys.modules` first and the
estimator becomes a function this file controls. That is what makes it possible
to test the thing that actually matters, which is not arithmetic.

WHAT ACTUALLY MATTERS HERE
--------------------------
`aletheia_many.py` exists only to avoid paying container start and interpreter
import once per image. It is worth nothing if it answers differently from
`aletheia_one.py`, and it is worse than nothing if it answers the same values
against the wrong images, because a misattributed score is plausible, passes
every other check, and gets published.

So the load bearing tests are:

- the two adapters produce the same value for the same image, and
- every line the batched one prints is keyed to the image it is about, even when
  the estimator fails on some of them.

The second is the one a positional protocol fails. These tests make an image in
the middle of a batch fail on purpose, which is the case where positional output
shifts every later answer onto the wrong file.
"""

import io
import pathlib
import sys
import types
import unittest
from contextlib import redirect_stdout, redirect_stderr

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import aletheia_many  # noqa: E402
import aletheia_one  # noqa: E402


class Boom(Exception):
    """Raised by a fake estimator to stand for one unreadable image."""


class FakeArray:
    """The smallest thing both adapters treat as an image.

    `ndim` of 2 is the greyscale route, which calls the estimator once with a
    channel of `None`. Anything else is the colour route, which calls it per
    channel and keeps the maximum.
    """

    def __init__(self, name: str, ndim: int = 2, channels: int = 3) -> None:
        self.name = name
        self.ndim = ndim
        self.shape = (8, 8) if ndim == 2 else (8, 8, channels)


def install_fakes(scores: dict, fail: set = frozenset()) -> None:
    """Put fake numpy, imageio and aletheialib in `sys.modules`.

    `scores` maps a path to the value the estimator returns for it. A path in
    `fail` raises instead, standing for an image that cannot be read.
    """
    numpy = types.ModuleType("numpy")
    # The real adapters use this to reject a non finite estimate, which is a
    # refusal both of them have to keep making.
    numpy.isfinite = lambda v: v == v and v not in (float("inf"), float("-inf"))

    imageio = types.ModuleType("imageio")

    def imread(path):
        if path in fail:
            raise Boom("cannot read it")
        return FakeArray(path)

    imageio.imread = imread

    attacks = types.ModuleType("aletheialib.attacks")

    def estimator(img, channel):
        return scores[img.name]

    attacks.spa_image = estimator
    attacks.rs_image = estimator
    aletheialib = types.ModuleType("aletheialib")
    aletheialib.attacks = attacks

    sys.modules["numpy"] = numpy
    sys.modules["imageio"] = imageio
    sys.modules["aletheialib"] = aletheialib
    sys.modules["aletheialib.attacks"] = attacks


def remove_fakes() -> None:
    for name in ("numpy", "imageio", "aletheialib", "aletheialib.attacks"):
        sys.modules.pop(name, None)


def run_one(path: str, method: str = "spa"):
    """`aletheia_one.py <path> <method>`, returning (exit code, stdout)."""
    out = io.StringIO()
    with redirect_stdout(out), redirect_stderr(io.StringIO()):
        code = aletheia_one.main(["aletheia_one.py", path, method])
    return code, out.getvalue()


def run_many(paths: list, method: str = "spa"):
    """`aletheia_many.py <method> <paths...>`, returning (code, stdout, stderr)."""
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = aletheia_many.main(["aletheia_many.py", method, *paths])
    return code, out.getvalue(), err.getvalue()


def keyed(stdout: str) -> dict:
    """Parse the batched protocol the way the Rust host does."""
    answers = {}
    for line in stdout.splitlines():
        if not line.strip():
            continue
        key, _, value = line.partition("\t")
        assert _ == "\t", f"line is not keyed: {line!r}"
        answers[key] = value
    return answers


class Base(unittest.TestCase):
    def tearDown(self) -> None:
        remove_fakes()


class TheTwoAdaptersAgree(Base):
    """The reason the batched one is allowed to exist."""

    def test_the_same_image_gets_the_same_value_from_both(self) -> None:
        scores = {"a.png": 0.0041024279, "b.png": -0.0022671186, "c.png": 0.5}
        for path, expected in scores.items():
            with self.subTest(path):
                install_fakes(scores)
                code_one, out_one = run_one(path)
                remove_fakes()
                install_fakes(scores)
                code_many, out_many, _ = run_many([path])
                self.assertEqual(code_one, 0)
                self.assertEqual(code_many, 0)
                # Byte for byte, not merely close. The two print through the
                # same format string and a difference here would mean one of
                # them had started rounding.
                self.assertEqual(keyed(out_many)[path], out_one.strip())
                self.assertEqual(float(out_one), expected)

    def test_a_whole_batch_matches_the_same_images_scored_one_at_a_time(self) -> None:
        scores = {f"{i}.png": i / 7 - 0.5 for i in range(8)}
        paths = list(scores)
        install_fakes(scores)
        _, batched, _ = run_many(paths)
        remove_fakes()
        singly = {}
        for path in paths:
            install_fakes(scores)
            _, out = run_one(path)
            remove_fakes()
            singly[path] = out.strip()
        self.assertEqual(keyed(batched), singly)

    def test_both_keep_a_negative_estimate(self) -> None:
        # The spread of small negatives on clean images is what a false positive
        # rate is measured from. Clamping it would make every clean image look
        # identical, so neither adapter may do it.
        install_fakes({"n.png": -0.0067568111})
        _, out_many, _ = run_many(["n.png"])
        self.assertEqual(keyed(out_many)["n.png"], "-0.0067568111")
        remove_fakes()
        install_fakes({"n.png": -0.0067568111})
        self.assertEqual(run_one("n.png")[1].strip(), "-0.0067568111")

    def test_both_take_the_maximum_across_colour_channels(self) -> None:
        # Aletheia's own CLI flags an image when ANY channel crosses its
        # threshold, so the maximum is what matches its shipped behaviour. The
        # mean would report a weaker detector than Aletheia is.
        seen = []

        install_fakes({})
        import imageio  # the fake

        imageio.imread = lambda p: FakeArray(p, ndim=3, channels=3)
        values = [0.1, 0.9, 0.2]
        sys.modules["aletheialib.attacks"].spa_image = lambda img, c: (
            seen.append(c) or values[c]
        )
        _, out, _ = run_many(["rgb.png"])
        self.assertEqual(float(keyed(out)["rgb.png"]), 0.9)
        self.assertEqual(seen, [0, 1, 2])


class EveryAnswerIsKeyedToItsOwnImage(Base):
    """The tests a positional protocol fails."""

    def test_an_image_that_fails_mid_batch_does_not_shift_the_others(self) -> None:
        scores = {"a.png": 0.1, "b.png": 0.2, "c.png": 0.3, "d.png": 0.4}
        install_fakes(scores, fail={"b.png"})
        code, out, err = run_many(list(scores))
        answers = keyed(out)
        # Three answers, each against the right image. Positionally, c and d
        # would have moved up one and been recorded against b and c.
        self.assertEqual(sorted(answers), ["a.png", "c.png", "d.png"])
        self.assertEqual(float(answers["c.png"]), 0.3)
        self.assertEqual(float(answers["d.png"]), 0.4)
        self.assertNotIn("b.png", answers)
        # The failure names the image it was about, so the one line a reader
        # sees tells them which file to go and look at.
        self.assertIn("b.png", err)
        self.assertIn("Boom", err)
        # Still a success: 63 good measurements must not be lost to one
        # unreadable file.
        self.assertEqual(code, 0)

    def test_order_is_preserved_but_the_key_is_what_identifies_an_answer(self) -> None:
        scores = {"x.png": 0.11, "y.png": 0.22}
        install_fakes(scores)
        _, out, _ = run_many(["y.png", "x.png"])
        lines = [line.split("\t")[0] for line in out.strip().splitlines()]
        self.assertEqual(lines, ["y.png", "x.png"])
        self.assertEqual(float(keyed(out)["x.png"]), 0.11)

    def test_the_key_is_the_path_exactly_as_given(self) -> None:
        # The Rust host mounts each image at a path it chose and maps the answer
        # back by that exact string, so any normalisation here would break the
        # join.
        path = "/work/000007.jpg"
        install_fakes({path: 0.5})
        _, out, _ = run_many([path])
        self.assertIn(path, keyed(out))

    def test_a_non_finite_estimate_is_reported_and_not_printed_as_a_score(self) -> None:
        install_fakes({"a.png": 0.1, "inf.png": float("inf")})
        code, out, err = run_many(["a.png", "inf.png"])
        self.assertEqual(sorted(keyed(out)), ["a.png"])
        self.assertIn("inf.png", err)
        self.assertIn("not finite", err)
        self.assertEqual(code, 0)


class RefusalsThatMustStayLoud(Base):
    def test_a_batch_where_nothing_could_be_scored_exits_non_zero(self) -> None:
        # Exiting zero here is how silence becomes a measurement: the host would
        # read an empty stdout as a tool that ran fine and found nothing.
        install_fakes({"a.png": 0.1, "b.png": 0.2}, fail={"a.png", "b.png"})
        code, out, err = run_many(["a.png", "b.png"])
        self.assertEqual(out, "")
        self.assertEqual(code, 1)
        self.assertIn("none of the 2 image(s)", err)

    def test_a_failed_import_exits_three_rather_than_printing_nothing_per_image(self) -> None:
        # The failure that once made a 2,000 image run exit zero and produce
        # nothing. It is about the container, not any one image, so it must not
        # look like a clean sweep.
        remove_fakes()
        code, out, err = run_many(["a.png"])
        self.assertEqual(out, "")
        self.assertEqual(code, 3)
        self.assertIn("not importable", err)

    def test_an_unknown_method_is_refused_by_both(self) -> None:
        install_fakes({"a.png": 0.1})
        self.assertEqual(run_many(["a.png"], method="rubbish")[0], 2)
        remove_fakes()
        install_fakes({"a.png": 0.1})
        self.assertEqual(run_one("a.png", method="rubbish")[0], 2)

    def test_too_few_arguments_is_refused_with_the_usage_line(self) -> None:
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = aletheia_many.main(["aletheia_many.py", "spa"])
        self.assertEqual(code, 2)
        self.assertIn("aletheia_many.py", err.getvalue())

    def test_both_methods_are_accepted(self) -> None:
        for method in ("spa", "rs"):
            with self.subTest(method):
                install_fakes({"a.png": 0.25})
                self.assertEqual(float(keyed(run_many(["a.png"], method)[1])["a.png"]), 0.25)
                remove_fakes()


if __name__ == "__main__":
    unittest.main()

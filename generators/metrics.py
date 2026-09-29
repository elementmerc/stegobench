#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Detection metrics for Python callers, computed by the stegobench binary.

    from metrics import metrics
    m = metrics([0.91, 0.02, 0.88, 0.05], [True, False, True, False],
                budgets=[0.01, 0.10])
    m["auc"]                 # 1.0
    m["tpr_at_fpr"][0.01]    # 1.0
    m["n_clean"], m["n_stego"]

`scores` is one number per image, higher meaning more like stego. `labels` is
one boolean per image, True for the images that really do hide something. The
two lists describe the same images in the same order; if they do not, this
refuses rather than measuring whichever is shorter.

`budgets` are false-alarm rates, as fractions: 0.01 is one clean image in a
hundred wrongly flagged. The detection rate at each one is the headline number
for a comparison, and accuracy is not: a detector facing a corpus that is
mostly clean can score 95% accuracy by answering "clean" every time.

WHY THIS SHELLS OUT INSTEAD OF DOING THE ARITHMETIC
---------------------------------------------------
There used to be two implementations of these metrics in this repository, one
in Rust and one here, and they disagreed at the boundary where an operating
point sits exactly on the false-alarm budget. Two answers to the same question,
with nothing in either output to say which one a reader was holding. So there
is now one implementation, in Rust, and this is the way into it. Python stays
a supported way to drive stegobench; it is no longer a second opinion on what
the numbers are.

There is deliberately no fallback that recomputes here when the binary is
absent. A fallback is the second implementation again, and it would be the copy
that runs on the machine where it is tested least.

WHEN IT FAILS
-------------
`MetricsUnavailable` means the stegobench binary could not be found, and names
every place that was tried and the line to type. `MetricsRefused` means the
binary ran and declined to produce a number; `.reason` is a stable word for
which condition fired, so a caller can branch on it without matching English
prose:

    one-sided        every image carries the same label, so there is nothing
                     to tell apart
    not-a-number     at least one score is not a number, so it cannot be ranked
    empty            no scores were given
    length-mismatch  the scores and the labels came from different record sets
    budget-not-a-rate    a budget is not a fraction between 0 and 1
    too-many-scores, input-too-large, not-json, duplicate-budget, read-failed
"""
from __future__ import annotations

import functools
import json
import math
import os
import pathlib
import shutil
import subprocess
import sys

#: Where `metrics.py` sits, so the checkout's own build can be found from it.
HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent

#: The binary this drives. The name is the crate's, not the package's.
BINARY_NAME = "stegobench"

#: Wall-clock ceiling on one call.
#:
#: The work is a sort and one pass, so the largest published arm (Pentimento
#: Core, 344,357 pairs, 688,714 answers) is seconds. Ten minutes is generous by
#: two orders of magnitude and exists to bound a process that has stopped
#: making progress rather than to pace a real one.
TIMEOUT_SECONDS = 600

#: The budgets reported when a caller names none. The same three a `result-v1`
#: document carries, so a figure from here and a figure from `stegobench score`
#: sit at the same operating points.
DEFAULT_BUDGETS = (0.01, 0.05, 0.10)


class MetricsUnavailable(RuntimeError):
    """The stegobench binary could not be found or could not be run."""


class MetricsRefused(ValueError):
    """The binary ran and declined to produce a number.

    `reason` is the stable word for which condition fired, `exit_code` the
    documented exit code, and the message is what the tool printed.
    """

    def __init__(self, reason: str, message: str, exit_code: int) -> None:
        super().__init__(message)
        self.reason = reason
        self.exit_code = exit_code


def _candidates() -> list[tuple[str, pathlib.Path | None]]:
    """Every place the binary is looked for, in order, without touching disk.

    Split out from `resolve_binary` so the order itself is testable, and so a
    failure can name each place rather than only the last.
    """
    named = os.environ.get("STEGOBENCH_BIN")
    found = shutil.which(BINARY_NAME)
    return [
        ("STEGOBENCH_BIN", pathlib.Path(named) if named else None),
        ("on PATH", pathlib.Path(found) if found else None),
        ("this checkout, release build", REPO / "target" / "release" / BINARY_NAME),
        ("this checkout, debug build", REPO / "target" / "debug" / BINARY_NAME),
    ]


@functools.lru_cache(maxsize=1)
def resolve_binary() -> tuple[pathlib.Path, str]:
    """The binary to run, and which candidate answered.

    Order: `STEGOBENCH_BIN`, then PATH, then this checkout's release build,
    then its debug build. A path named in the environment is used as given, so
    a path that is not there is an error rather than a fall back to something
    else: somebody who set the variable has said which binary they mean.

    A debug build is the last resort and says so on stderr when it is the one
    that answered, because it is the build most likely to be stale and a number
    from a binary nobody meant to run is exactly what this module exists to
    prevent.
    """
    tried: list[str] = []
    for where, path in _candidates():
        if path is None:
            tried.append(f"{where}: nothing")
            continue
        if path.is_file() and os.access(path, os.X_OK):
            if where.endswith("debug build"):
                print(
                    f"metrics: using the debug build at {path}. It is the last "
                    f"place looked and may be older than your source; "
                    f"`cargo build --release -p stegobench-cli` builds the one "
                    f"preferred over it.",
                    file=sys.stderr,
                )
            return path, where
        tried.append(f"{where}: {path}")
    raise MetricsUnavailable(
        "no stegobench binary, so there is nothing to compute these metrics "
        "with. This deliberately has no Python fallback: a second "
        "implementation of a metric is a second answer to the same question.\n"
        "  Looked in, in order:\n    " + "\n    ".join(tried) + "\n"
        "  Build it here:  cargo build --release -p stegobench-cli\n"
        "  Or name one:    export STEGOBENCH_BIN=/path/to/stegobench"
    )


def _encode(scores, labels) -> str:
    """The request document, with the one value JSON cannot carry spelled out.

    A score of NaN is a detector that produced no number for that image, and
    JSON has no NaN, so it travels as `null` and the binary refuses it by name.
    An infinite score is a different thing: it orders perfectly well, JSON
    still cannot carry it, and turning it into `null` would quietly convert a
    real ordering into an absence. So it is refused here instead.
    """
    out: list[float | None] = []
    for i, score in enumerate(scores):
        value = float(score)
        if math.isnan(value):
            out.append(None)
        elif math.isinf(value):
            raise MetricsRefused(
                "not-representable",
                f"score {i} is {value}, which JSON cannot carry. An infinite "
                f"score orders fine and cannot be sent, and sending it as "
                f"'no answer' would turn a real ranking into a missing one.",
                2,
            )
        else:
            out.append(value)
    flags = []
    for i, label in enumerate(labels):
        if isinstance(label, bool):
            flags.append(label)
        elif isinstance(label, int) and label in (0, 1):
            flags.append(bool(label))
        else:
            raise MetricsRefused(
                "label-not-a-flag",
                f"label {i} is {label!r}. A label says whether that image "
                f"really hides something, so it is True or False.",
                2,
            )
    return json.dumps({"scores": out, "labels": flags})


def metrics(scores, labels, budgets=DEFAULT_BUDGETS) -> dict:
    """ROC AUC and the detection rate at each false-alarm budget.

    Returns `{"auc": float, "tpr_at_fpr": {budget: float}, "n_clean": int,
    "n_stego": int}`, with `tpr_at_fpr` keyed by the budgets as given, so a
    caller looks up the float it passed rather than a string this chose.

    Raises `MetricsRefused` when the numbers cannot honestly be ranked, and
    `MetricsUnavailable` when the binary is not there. It never returns a
    partial answer: either every budget was computed or none was.
    """
    budgets = list(budgets)
    if not budgets:
        raise MetricsRefused(
            "no-budgets",
            "no false-alarm budgets given, so there is no detection rate to "
            "report. Pass at least one, such as 0.01 for one per cent.",
            2,
        )
    # Keyed by the text sent, so the answer can be matched back to the float
    # the caller handed over whatever `repr` does to it.
    texts: dict[str, float] = {}
    for budget in budgets:
        text = repr(float(budget))
        if text in texts:
            raise MetricsRefused(
                "duplicate-budget",
                f"a false-alarm budget of {budget} was given twice. One budget "
                f"is one measurement.",
                2,
            )
        texts[text] = budget

    binary, _ = resolve_binary()
    body = _encode(scores, labels)
    argv = [str(binary), "--json", "metrics", "-"]
    for text in texts:
        argv += ["--at", text]
    try:
        run = subprocess.run(
            argv,
            input=body,
            capture_output=True,
            text=True,
            timeout=TIMEOUT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired as e:
        raise MetricsUnavailable(
            f"{binary} did not answer within {TIMEOUT_SECONDS} seconds over "
            f"{len(labels)} label(s), so it was killed. This is a bug in "
            f"stegobench rather than anything you did; please report it."
        ) from e
    except OSError as e:
        raise MetricsUnavailable(f"could not run {binary}: {e}") from e

    try:
        payload = json.loads(run.stdout)
    except json.JSONDecodeError as e:
        # Fail loud and hand over both streams. A tool that exited non-zero
        # without machine-readable output has failed in a way this module
        # cannot describe, and swallowing it would report that as no answer.
        raise MetricsUnavailable(
            f"{binary} exited {run.returncode} and did not write a JSON "
            f"answer: {e}\n  stdout: {run.stdout.strip()[:400]}\n"
            f"  stderr: {run.stderr.strip()[:400]}"
        ) from e

    if run.returncode != 0 or not payload.get("ok"):
        raise MetricsRefused(
            payload.get("reason", "unknown"),
            payload.get("error", run.stderr.strip() or "no reason given"),
            run.returncode,
        )
    return {
        "auc": payload["auc"],
        "tpr_at_fpr": {texts[t]: v for t, v in payload["tpr_at_fpr"].items()},
        "n_clean": payload["n_clean"],
        "n_stego": payload["n_stego"],
    }

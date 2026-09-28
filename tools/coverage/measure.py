#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Measure test coverage for either half of this repository, and hold a floor.

Two halves, one command each, both run from the repository root:

    python3 tools/coverage/measure.py python
    python3 tools/coverage/measure.py rust

Each prints a per-file table, a total per component, and whether the total sits
at or above the floor recorded in `tools/coverage/floors.toml`. Below the
floor is a failure and exit code 1; at or above it is a pass. `--bump` rewrites
the floors to what was just measured, which is the only way a floor moves and
is a deliberate commit rather than something CI does behind anybody's back.

Why a ratchet rather than a flat 90% gate: see `tools/coverage/README.md`. The
short version is that the 90% rule in CLAUDE.md Section 7 applies to CHANGED
files, a whole-tree gate at 90% would be red on its first run for code nobody
is touching, and a check that is permanently red is a check people learn to
scroll past.

Exit codes:
    0   every component at or above its floor
    1   at least one component below its floor
    2   the measurement could not be made (tool missing, tests failed, no data)
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
TOOL_DIR = REPO_ROOT / "tools" / "coverage"
FLOORS_PATH = TOOL_DIR / "floors.toml"
COVERAGERC = TOOL_DIR / "coveragerc"

# The per-file bar CLAUDE.md Section 7 asks for. It is reported, not enforced;
# what is enforced is the floor. See README.md for why.
CHANGED_FILE_BAR = 90.0

# Every subprocess here is bounded. A cold Rust build with instrumentation is
# the long pole; the Python suites are seconds. Both are overridable for a
# slower machine rather than being unbounded.
PYTHON_TIMEOUT_S = int(os.environ.get("COVERAGE_PYTHON_TIMEOUT_S", "1800"))
RUST_TIMEOUT_S = int(os.environ.get("COVERAGE_RUST_TIMEOUT_S", "5400"))

# The Python suites, each discovered exactly the way CI discovers them.
PYTHON_SUITES = (
    ("generators", "generators"),
    ("tools/release", "tools/release"),
)

# Measured one crate at a time rather than `--workspace`, because a workspace
# build with coverage instrumentation is heavy enough to be killed by a memory
# limit on a developer machine. `cargo llvm-cov report` merges the profiles
# afterwards, so the combined figure is the same one a workspace run produces.
RUST_CRATES = (
    "stegobench-core",
    "stegobench-metrics",
    "stegobench-plugin",
    "stegobench-cli",
)


@dataclass(frozen=True)
class FileCoverage:
    """One source file's figure, as a percentage of whatever the half counts."""

    path: str
    covered: int
    total: int

    @property
    def percent(self) -> float:
        if self.total == 0:
            return 100.0
        return 100.0 * self.covered / self.total


@dataclass
class Component:
    """One thing that carries its own floor: a Python suite, or the workspace."""

    name: str
    unit: str
    files: list[FileCoverage]

    @property
    def covered(self) -> int:
        return sum(f.covered for f in self.files)

    @property
    def total(self) -> int:
        return sum(f.total for f in self.files)

    @property
    def percent(self) -> float:
        if self.total == 0:
            return 100.0
        return 100.0 * self.covered / self.total


class MeasurementError(RuntimeError):
    """Raised when the number could not be produced, never when it is low."""


def run(
    cmd: list[str],
    *,
    timeout: int,
    cwd: Path = REPO_ROOT,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess:
    try:
        return subprocess.run(
            cmd,
            cwd=cwd,
            env=env,
            timeout=timeout,
            capture_output=True,
            text=True,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        raise MeasurementError(
            f"`{' '.join(cmd)}` did not finish within {timeout} seconds. "
            f"Raise COVERAGE_PYTHON_TIMEOUT_S or COVERAGE_RUST_TIMEOUT_S if the "
            f"machine is simply slow, rather than assuming it hung."
        ) from exc
    except OSError as exc:
        raise MeasurementError(f"`{cmd[0]}` could not be started: {exc}") from exc


# ── The Python half ─────────────────────────────────────────────────────────


def measure_python(work_dir: Path) -> list[Component]:
    try:
        import coverage  # noqa: F401
    except ImportError as exc:
        raise MeasurementError(
            "coverage.py is not installed for this interpreter. Install it with\n"
            f"    {sys.executable} -m pip install coverage\n"
            "or run this inside the virtual environment the tests use."
        ) from exc

    data_file = work_dir / "coverage-data"
    json_path = work_dir / "coverage.json"
    for stale in work_dir.glob("coverage-data*"):
        stale.unlink()

    env = dict(os.environ)
    env["COVERAGE_FILE"] = str(data_file)

    for label, directory in PYTHON_SUITES:
        print(f"  running the {label} suite under coverage", flush=True)
        proc = subprocess.run(
            [
                sys.executable,
                "-m",
                "coverage",
                "run",
                f"--rcfile={COVERAGERC}",
                "-m",
                "unittest",
                "discover",
                "-s",
                directory,
                "-p",
                "test_*.py",
            ],
            cwd=REPO_ROOT,
            env=env,
            timeout=PYTHON_TIMEOUT_S,
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            tail = "\n".join((proc.stderr or proc.stdout).splitlines()[-25:])
            raise MeasurementError(
                f"the {label} suite failed, so its coverage figure would be "
                f"measuring a broken tree and is not reported. Last lines:\n{tail}"
            )

    combine = run(
        [sys.executable, "-m", "coverage", "combine", f"--rcfile={COVERAGERC}"],
        timeout=PYTHON_TIMEOUT_S,
        env=env,
    )
    if combine.returncode != 0:
        raise MeasurementError(f"coverage combine failed:\n{combine.stderr.strip()}")

    report = subprocess.run(
        [
            sys.executable,
            "-m",
            "coverage",
            "json",
            f"--rcfile={COVERAGERC}",
            "-o",
            str(json_path),
        ],
        cwd=REPO_ROOT,
        env=env,
        timeout=PYTHON_TIMEOUT_S,
        capture_output=True,
        text=True,
        check=False,
    )
    if report.returncode != 0:
        raise MeasurementError(f"coverage json failed:\n{report.stderr.strip()}")

    payload = json.loads(json_path.read_text(encoding="utf-8"))
    files = payload.get("files")
    if not files:
        raise MeasurementError(
            "coverage produced no per-file data. That means the suites imported "
            "nothing under `source`, which is a configuration fault rather than "
            "a coverage of zero."
        )

    buckets: dict[str, list[FileCoverage]] = {label: [] for label, _ in PYTHON_SUITES}
    for raw_path, entry in files.items():
        path = raw_path.replace("\\", "/")
        summary = entry["summary"]
        # coverage.py's own blended figure: statements plus branch outcomes.
        covered = summary["covered_lines"] + summary["covered_branches"]
        total = summary["num_statements"] + summary["num_branches"]
        for label, directory in PYTHON_SUITES:
            if path.startswith(f"{directory}/"):
                buckets[label].append(FileCoverage(path, covered, total))
                break

    return [
        Component(label, "statements and branches", sorted(buckets[label], key=lambda f: f.path))
        for label, _ in PYTHON_SUITES
        if buckets[label]
    ]


# ── The Rust half ───────────────────────────────────────────────────────────


def measure_rust(work_dir: Path) -> list[Component]:
    if shutil.which("cargo") is None:
        raise MeasurementError("cargo is not on PATH.")
    if shutil.which("cargo-llvm-cov") is None:
        raise MeasurementError(
            "cargo-llvm-cov is not installed. Install it with\n"
            "    cargo install cargo-llvm-cov --locked\n"
            "and make sure the `llvm-tools` rustup component is present:\n"
            "    rustup component add llvm-tools-preview"
        )

    clean = run(["cargo", "llvm-cov", "clean", "--workspace"], timeout=RUST_TIMEOUT_S)
    if clean.returncode != 0:
        raise MeasurementError(f"cargo llvm-cov clean failed:\n{clean.stderr.strip()}")

    for crate in RUST_CRATES:
        print(f"  running the {crate} tests under instrumentation", flush=True)
        proc = run(
            ["cargo", "llvm-cov", "--no-report", "--package", crate],
            timeout=RUST_TIMEOUT_S,
        )
        if proc.returncode != 0:
            tail = "\n".join((proc.stderr or proc.stdout).splitlines()[-25:])
            raise MeasurementError(
                f"the {crate} tests failed, so the figure would be measuring a "
                f"broken tree and is not reported. Last lines:\n{tail}"
            )

    report = run(["cargo", "llvm-cov", "report", "--json"], timeout=RUST_TIMEOUT_S)
    if report.returncode != 0:
        raise MeasurementError(f"cargo llvm-cov report failed:\n{report.stderr.strip()}")

    payload = json.loads(report.stdout)
    data = payload.get("data")
    if not data or not data[0].get("files"):
        raise MeasurementError(
            "cargo llvm-cov produced no per-file data. That is a missing profile "
            "rather than a coverage of zero."
        )

    files: list[FileCoverage] = []
    for entry in data[0]["files"]:
        raw = Path(entry["filename"])
        try:
            path = raw.relative_to(REPO_ROOT).as_posix()
        except ValueError:
            # A dependency compiled from outside the tree. Not ours to measure.
            continue
        regions = entry["summary"]["regions"]
        files.append(FileCoverage(path, regions["covered"], regions["count"]))

    if not files:
        raise MeasurementError("no workspace source files appeared in the coverage report.")

    return [Component("rust", "regions", sorted(files, key=lambda f: f.path))]


# ── Floors ──────────────────────────────────────────────────────────────────


def load_floors() -> dict[str, float]:
    if not FLOORS_PATH.exists():
        raise MeasurementError(
            f"{FLOORS_PATH} is missing. It is committed on purpose: without it "
            f"there is no record of what the floor was, and a ratchet with no "
            f"record is a number somebody can quietly lower."
        )
    try:
        parsed = tomllib.loads(FLOORS_PATH.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        raise MeasurementError(f"{FLOORS_PATH} is not valid TOML: {exc}") from exc

    floors: dict[str, float] = {}
    for name, entry in parsed.get("component", {}).items():
        if not isinstance(entry, dict) or "floor" not in entry:
            raise MeasurementError(f"{FLOORS_PATH}: component `{name}` has no `floor`.")
        floor = entry["floor"]
        if not isinstance(floor, (int, float)) or not 0 <= floor <= 100:
            raise MeasurementError(
                f"{FLOORS_PATH}: component `{name}` has a floor of {floor!r}, "
                f"which is not a percentage between 0 and 100."
            )
        floors[name] = float(floor)
    if not floors:
        raise MeasurementError(f"{FLOORS_PATH} records no components.")
    return floors


def bump_floors(components: list[Component]) -> None:
    """Raise the recorded floors to the measured figures. Never lowers one."""
    text = FLOORS_PATH.read_text(encoding="utf-8")
    floors = load_floors()
    changed = []
    for component in components:
        new = float(int(component.percent))  # whole percent, always rounded down
        old = floors.get(component.name)
        if old is None:
            # A component the file has never heard of. Skipping it silently is
            # how a newly added crate or suite ends up measured but never
            # gated: `render` shows it with no floor, `--bump` passes over it,
            # and nothing ever says so. Recording it is the whole point of a
            # ratchet, so it is recorded, loudly, with its name quoted only
            # where TOML requires it.
            key = component.name if component.name.replace("_", "").replace(
                "-", "").isalnum() else f'"{component.name}"'
            if not text.endswith("\n"):
                text += "\n"
            text += f"\n[component.{key}]\nfloor = {new:g}\n"
            changed.append((component.name, None, new))
            continue
        if new <= old:
            continue
        # TOML allows a bare key or a quoted one, and this file uses both:
        # `tools/release` has to be quoted, `generators` does not.
        candidates = [f"[component.{component.name}]", f'[component."{component.name}"]']
        needle = next((c for c in candidates if c in text), None)
        if needle is None:
            raise MeasurementError(
                f"{FLOORS_PATH}: cannot find a header for `{component.name}` to "
                f"rewrite. Tried {' and '.join(candidates)}."
            )
        head, _, tail = text.partition(needle)
        old_line = f"floor = {old:g}"
        if old_line not in tail:
            raise MeasurementError(
                f"{FLOORS_PATH}: `{old_line}` is not written the way this rewriter "
                f"expects under {needle}. Edit the floor by hand."
            )
        tail = tail.replace(old_line, f"floor = {new:g}", 1)
        text = head + needle + tail
        changed.append((component.name, old, new))

    if not changed:
        print("Floors already match the measurement. Nothing rewritten.")
        return

    tmp = FLOORS_PATH.with_suffix(".toml.part")
    tmp.write_text(text, encoding="utf-8")
    tmp.replace(FLOORS_PATH)  # atomic, so an interrupted bump leaves no half file
    for name, old, new in changed:
        if old is None:
            print(f"Recorded a first floor of {new:g}% for {name}, which this "
                  f"file had never heard of.")
        else:
            print(f"Raised the {name} floor from {old:g}% to {new:g}%.")
    print("Commit tools/coverage/floors.toml so the new floor is the recorded one.")


# ── Reporting ───────────────────────────────────────────────────────────────


def render(components: list[Component], floors: dict[str, float]) -> tuple[str, bool]:
    """Return the Markdown report and whether every component met its floor."""
    lines: list[str] = ["## Test coverage", ""]
    ok = True

    lines.append("| Component | Measured | Floor | Unit | Verdict |")
    lines.append("|---|---:|---:|---|---|")
    for component in components:
        floor = floors.get(component.name)
        if floor is None:
            verdict = "no floor recorded"
            ok = False
        elif component.percent + 1e-9 >= floor:
            verdict = "at or above the floor"
        else:
            verdict = "**below the floor**"
            ok = False
        floor_text = "none" if floor is None else f"{floor:g}%"
        lines.append(
            f"| `{component.name}` | {component.percent:.2f}% | {floor_text} "
            f"| {component.unit} | {verdict} |"
        )
    lines.append("")

    for component in components:
        below = [f for f in component.files if f.percent < CHANGED_FILE_BAR]
        lines.append(
            f"### `{component.name}`: {len(below)} of {len(component.files)} files "
            f"below {CHANGED_FILE_BAR:g}%"
        )
        lines.append("")
        if not below:
            lines.append(f"Every file is at or above {CHANGED_FILE_BAR:g}%.")
            lines.append("")
            continue
        lines.append(
            "CLAUDE.md Section 7 asks for 90% on CHANGED files. This list is "
            "advisory, not a gate: if your change touches one of these, that "
            "is the row to fix before the work is done."
        )
        lines.append("")
        lines.append("| File | Covered | Total | Coverage |")
        lines.append("|---|---:|---:|---:|")
        for f in sorted(below, key=lambda f: (f.percent, f.path)):
            lines.append(f"| `{f.path}` | {f.covered} | {f.total} | {f.percent:.2f}% |")
        lines.append("")

    headroom = [
        (c.name, floors[c.name], c.percent)
        for c in components
        if c.name in floors and int(c.percent) > floors[c.name]
    ]
    if headroom:
        lines.append("### The floor can be raised")
        lines.append("")
        for name, floor, measured in headroom:
            lines.append(
                f"- `{name}` — measured {measured:.2f}%, floor {floor:g}%. "
                f"Run `python3 tools/coverage/measure.py <half> --bump` and commit."
            )
        lines.append("")

    return "\n".join(lines), ok


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Measure coverage for one half of the repository and check its floor.",
    )
    parser.add_argument(
        "half",
        choices=("python", "rust"),
        help="`python` runs the generators and release suites; `rust` runs the workspace.",
    )
    parser.add_argument(
        "--bump",
        action="store_true",
        help="Raise the recorded floors to what was just measured. Never lowers one.",
    )
    parser.add_argument(
        "--summary",
        type=Path,
        help="Append the Markdown report to this file, typically $GITHUB_STEP_SUMMARY.",
    )
    args = parser.parse_args()

    work_dir = REPO_ROOT / "target" / "coverage"
    work_dir.mkdir(parents=True, exist_ok=True)

    try:
        floors = load_floors()
        print(f"Measuring the {args.half} half. This runs the real test suites.", flush=True)
        components = measure_python(work_dir) if args.half == "python" else measure_rust(work_dir)
    except MeasurementError as exc:
        print(f"\nCoverage was NOT measured: {exc}", file=sys.stderr)
        return 2

    report, ok = render(components, floors)
    print()
    print(report)

    if args.summary is not None:
        try:
            with args.summary.open("a", encoding="utf-8") as handle:
                handle.write(report + "\n")
        except OSError as exc:
            # A summary that cannot be written must not lose the verdict.
            print(f"Could not write the summary to {args.summary}: {exc}", file=sys.stderr)

    if args.bump:
        try:
            bump_floors(components)
        except MeasurementError as exc:
            print(f"Floors were NOT rewritten: {exc}", file=sys.stderr)
            return 2
        return 0

    if not ok:
        print(
            "\nCoverage fell below a recorded floor. That is a regression in the "
            "tests rather than a threshold to raise: find what stopped being "
            "exercised. If the drop is deliberate, lower the floor in "
            "tools/coverage/floors.toml by hand, in its own commit, with the "
            "reason in the commit message.",
            file=sys.stderr,
        )
        return 1

    print("\nEvery component is at or above its floor.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

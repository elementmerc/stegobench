#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""One way in to forty-two programs.

WHY
---
Until now the only way to use any of this was to clone the repository, work out
which of forty-two files does the thing you want, guess the dependency set, and
run `python generators/whichever.py`. That is a pile of scripts rather than a
tool, and the difference is most of what decides whether anybody else ever uses
it.

WHY IT IS NOT CALLED `stegobench`
---------------------------------
The v0.1 plan asks for "a single `stegobench` console command wrapping the
existing scripts as subcommands". That name is taken: `crates/stegobench-cli`
already installs a binary called `stegobench`, and two different programs of
one name on one PATH is a worse problem than the one being solved.

The plan predates the Rust workspace, and the split it describes elsewhere
resolves it. **stegobench is the benchmark. Pentimento is the corpus.** Every
program in this directory builds, audits, packs or verifies the corpus, so the
corpus's name is the honest one for them, and the two tools stay
distinguishable on a PATH and in somebody's shell history.

HOW IT FINDS SUBCOMMANDS
------------------------
By import, not by a hand-kept list. A list would drift the moment somebody adds
a file, and this directory reached forty-two files while its README described
three. A module belongs here when it defines `main(argv)`; its first docstring
line becomes its help text, so a subcommand cannot be added without describing
itself.

    pentimento                      list the subcommands
    pentimento verify-release --help
    pentimento verify-release --covers ~/pentimento/covers/commons

Underscores and hyphens are interchangeable, because nobody remembers which a
given tool chose.
"""
from __future__ import annotations

import importlib
import pathlib
import pkgutil
import sys

HERE = pathlib.Path(__file__).resolve().parent

#: Not subcommands. `cli` is this file; the rest are libraries or test modules.
EXCLUDED = {"cli", "__main__"}


def candidates() -> list[str]:
    """Every module in this directory that could be a subcommand."""
    return sorted(
        m.name for m in pkgutil.iter_modules([str(HERE)])
        if m.name not in EXCLUDED and not m.name.startswith("test_")
    )


class Unavailable:
    """A subcommand that exists but cannot run here, and why.

    A MISSING COMMAND AND A BROKEN ONE MUST NOT RENDER THE SAME.

    Several modules exit at import when an optional dependency is absent,
    which is right when they are run and wrong when they are merely listed:
    dropping them leaves a user comparing the listing against the README and
    concluding the tool is out of date, when the truth is that `conseal` is
    not installed. An absence explains nothing; a named command with a reason
    beside it explains everything and says what to do.
    """

    def __init__(self, name: str, reason: str) -> None:
        self.name = name
        self.reason = reason


def describe(name: str) -> tuple[object, str] | Unavailable | None:
    """The module and its summary, an Unavailable, or None if it is a library."""
    try:
        module = importlib.import_module(name)
    except SystemExit:
        return Unavailable(name, "a dependency it needs is not installed")
    except ImportError as e:
        return Unavailable(name, f"{e.name or e} is not installed")
    except Exception as e:  # noqa: BLE001 - the message is the point
        return Unavailable(name, f"{type(e).__name__}: {e}")
    if not callable(getattr(module, "main", None)):
        return None
    doc = (module.__doc__ or "").strip().splitlines()
    return module, (doc[0] if doc else "(no description)")


def normalise(name: str) -> str:
    return name.replace("-", "_")


def listing() -> int:
    # Every module is imported before anything is printed, so the modules that
    # complain on the way in do not interleave with the listing itself.
    described = {name: describe(name) for name in candidates()}

    print("pentimento: build, audit, pack and verify the Pentimento corpus\n")
    print("  pentimento <command> [options]")
    print("  pentimento <command> --help\n")

    runnable = {n: d for n, d in described.items() if isinstance(d, tuple)}
    for name, (_, summary) in sorted(runnable.items()):
        print(f"  {name.replace('_', '-'):28} {summary[:74]}")

    broken = {n: d for n, d in described.items() if isinstance(d, Unavailable)}
    if broken:
        print(f"\n{len(broken)} command(s) are present but cannot run here:")
        for name, entry in sorted(broken.items()):
            print(f"  {name.replace('_', '-'):28} {entry.reason}")
        print("  Install the dependencies:  pip install -r requirements.txt")

    if not runnable:
        print("\nno subcommands could be imported, which means this "
              "installation is broken rather than empty", file=sys.stderr)
        return 1
    print(f"\n{len(runnable)} commands. See generators/README.md for the "
          f"order they run in.")
    return 0


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    sys.path.insert(0, str(HERE))

    if not argv or argv[0] in ("-h", "--help", "help", "list"):
        return listing()

    wanted = normalise(argv[0])
    if wanted not in candidates():
        close = [n for n in candidates() if wanted in n or n in wanted]
        print(f"no such command: {argv[0]}", file=sys.stderr)
        if close:
            print(f"did you mean: "
                  f"{', '.join(n.replace('_', '-') for n in close[:3])}",
                  file=sys.stderr)
        else:
            print("run `pentimento` with no arguments for the list",
                  file=sys.stderr)
        return 2

    described = describe(wanted)
    if isinstance(described, Unavailable):
        print(f"{argv[0]} is present but cannot run here: {described.reason}.",
              file=sys.stderr)
        print("Install the dependencies:  pip install -r requirements.txt",
              file=sys.stderr)
        return 2
    if described is None:
        print(f"{argv[0]} is a library rather than a command. Run "
              f"`pentimento` for the list.", file=sys.stderr)
        return 2
    module, _ = described
    return module.main(argv[1:])


if __name__ == "__main__":
    raise SystemExit(main())

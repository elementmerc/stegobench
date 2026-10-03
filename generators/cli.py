#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""One way in to forty-six programs.

WHY
---
Until now the only way to use any of this was to clone the repository, work out
which of forty-six files does the thing you want, guess the dependency set, and
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
    pentimento --version            which toolkit built a corpus
    pentimento verify-release --help
    pentimento verify-release --covers ~/pentimento/covers/commons

Underscores and hyphens are interchangeable, because nobody remembers which a
given tool chose.

WHAT THIS FILE ADDS TO EVERY SUBCOMMAND
---------------------------------------
Three conventions that would otherwise have to be repeated in thirty-eight
parsers, and would drift in the thirty-ninth: the usage line names the
subcommand, each option states its default, and a build command whose numba
does not match `requirements.lock` says so before it starts.
"""
from __future__ import annotations

import argparse
import contextlib
import importlib
import io
import pathlib
import pkgutil
import sys
import textwrap

HERE = pathlib.Path(__file__).resolve().parent

#: Not subcommands. `cli` is this file; the rest are libraries or test modules.
EXCLUDED = {"cli", "__main__"}

#: How wide a summary may be in the listing before it is shortened.
SUMMARY_WIDTH = 74

#: The name on the index, which is not the name of the command. Kept in step
#: with `pyproject.toml` by `test_version_distribution_name`.
DISTRIBUTION = "pentimento-corpus"


def version() -> str:
    """The installed version of this package, or a marker that it is not one.

    A corpus is built over days and quoted for years, so the question "which
    toolkit produced this?" has to have an answer that does not depend on
    somebody remembering. Read from the installed distribution metadata rather
    than duplicated here, so `pyproject.toml` stays the only place it is
    written down. `0+unknown` is what a source tree that was never installed
    reports, which is a truthful answer rather than a failure.

    The distribution is NOT named for the command: it is `pentimento-corpus`,
    because `pentimento` on PyPI belongs to an unrelated project. Asking the
    metadata for the wrong name fails exactly the way an uninstalled checkout
    does, so a drifted name here would report `0+unknown` on a correctly
    installed package and stamp that into every corpus built with it. That is
    why `DISTRIBUTION` is checked against `pyproject.toml` by a test rather
    than being trusted to stay right.
    """
    from importlib import metadata

    try:
        return metadata.version(DISTRIBUTION)
    except metadata.PackageNotFoundError:
        return "0+unknown"


def edit_distance(a: str, b: str) -> int:
    """Levenshtein distance, two rows at a time."""
    if a == b:
        return 0
    previous = list(range(len(b) + 1))
    for i, ca in enumerate(a, start=1):
        current = [i]
        for j, cb in enumerate(b, start=1):
            current.append(min(previous[j] + 1, current[j - 1] + 1,
                               previous[j - 1] + (ca != cb)))
        previous = current
    return previous[-1]


def nearest(typed: str, names: list[str]) -> list[str]:
    """The commands `typed` was probably meant to be, best first, or nothing.

    Two relationships count, and nothing else does. A typo, judged by the same
    rule the Rust half applies: at most two edits, and no more than half the
    longer of the two words. Or an abbreviation, where what was typed is the
    start of a longer name, which is what somebody does when they remember
    half a command.

    Everything else gets silence. A confident wrong suggestion is worse than
    none, because it sends somebody to a command that does something else.
    """
    ordered = sorted(names)
    scored = [(edit_distance(typed, name), name) for name in ordered]
    close = sorted((d, n) for d, n in scored
                   if d <= 2 and d * 2 <= max(len(n), len(typed)))
    if close:
        return [n for _, n in close[:3]]
    return [n for n in ordered if typed and n.startswith(typed)][:3]


#: Commands whose output pixels come out of code `conseal` compiles with
#: numba. Which pixel carries a bit is decided by JIT-compiled machine code, so
#: a different compiler can move a cost by an ulp and change the answer. Every
#: other command reads or checks what these wrote, and is unaffected.
JIT_SENSITIVE = {
    "build_adaptive_arms", "build_core_tier", "build_jpeg_arms",
    "embed_adaptive", "payloadsweep", "sizesweep",
}

#: The packages the lock exists for. Named rather than compared wholesale: a
#: lock mismatch in `tqdm` is not a reason to warn somebody about their pixels.
LOCKED_COMPILERS = ("numba", "llvmlite")


def locked_versions() -> dict[str, str]:
    """What `requirements.lock` pins the compiler stack to, if it is readable.

    Absent when the package was installed from a wheel rather than from a
    checkout, and that is not an error: an unreadable lock means the check
    cannot be made, not that the environment is wrong.
    """
    lock = HERE.parent / "requirements.lock"
    try:
        text = lock.read_text(encoding="utf-8")
    except OSError:
        return {}
    pinned = {}
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if "==" not in line:
            continue
        name, _, value = line.partition("==")
        if name.strip().lower() in LOCKED_COMPILERS:
            pinned[name.strip().lower()] = value.strip()
    return pinned


def warn_if_unlocked(command: str, rest: list[str] | None = None) -> None:
    """Say so, once and on stderr, when a build runs outside the locked set.

    Not when the reader only asked what the command does: a warning about an
    environment nothing is about to use is noise, and noise is how a warning
    stops being read.
    """
    if command not in JIT_SENSITIVE:
        return
    if rest and ({"-h", "--help"} & set(rest)):
        return
    pinned = locked_versions()
    if not pinned:
        return
    from importlib import metadata
    drifted = []
    for name, wanted in sorted(pinned.items()):
        try:
            found = metadata.version(name)
        except Exception:  # noqa: BLE001 - absent is as interesting as different
            found = "absent"
        if found != wanted:
            drifted.append(f"{name} {found}, locked at {wanted}")
    if not drifted:
        return
    print(f"warning: {'; '.join(drifted)}", file=sys.stderr)
    print("  fix: pip install -r requirements.lock", file=sys.stderr)


class SubcommandHelp(argparse.HelpFormatter):
    """Help that states an option's default, wherever there is one to state.

    Every generator builds its own parser, and none of them repeated the
    default into its help text, so reading `--help` told you which dials exist
    and nothing about where they are set. Doing it here rather than in
    thirty-eight parsers means it cannot be half-applied.

    Narrower than `ArgumentDefaultsHelpFormatter`, which prints `(default:
    None)` against every optional that has no default and `(default: False)`
    against every flag: both are noise, and an option nobody can read past is
    no better than one that says nothing.
    """

    def _get_help_string(self, action):
        text = action.help or ""
        if action.nargs == 0 or not action.option_strings:
            return text
        if action.default is None or action.default is argparse.SUPPRESS:
            return text
        if "%(default)" in text:
            return text
        note = "(default: %(default)s)"
        return f"{text} {note}" if text else note


def with_defaults_in_help() -> None:
    """Make `SubcommandHelp` the formatter every subcommand gets for free.

    Patching the constructor rather than editing each parser is deliberate: a
    convention that has to be repeated thirty-eight times is one that drifts
    the first time somebody adds a thirty-ninth. A parser that chose its own
    formatter keeps it.
    """
    if getattr(argparse.ArgumentParser, "_pentimento_defaults", False):
        return
    original_init = argparse.ArgumentParser.__init__
    original_add = argparse._ActionsContainer.add_argument

    def patched_init(self, *args, **kwargs):
        kwargs.setdefault("formatter_class", SubcommandHelp)
        original_init(self, *args, **kwargs)

    def patched_add(self, *args, **kwargs):
        # argparse prints nothing at all for an option whose help is empty, so
        # the formatter never sees it. An option with a default and no prose
        # gets the default as its whole description rather than a blank line.
        action = original_add(self, *args, **kwargs)
        if (action.help is None and action.option_strings
                and action.nargs != 0
                and action.default is not None
                and action.default is not argparse.SUPPRESS):
            action.help = "(default: %(default)s)"
        return action

    argparse.ArgumentParser.__init__ = patched_init
    argparse._ActionsContainer.add_argument = patched_add
    argparse.ArgumentParser._pentimento_defaults = True


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
    """The module and its summary, an Unavailable, or None if it is a library.

    A module that exits at import prints its own complaint on the way out, and
    listing thirty-eight of them printed those complaints above the listing's
    own header, where they read as a crash rather than as a note. The reason is
    captured and shown beside the command instead, which is where somebody
    reading a list of commands will look for it.
    """
    try:
        with contextlib.redirect_stderr(io.StringIO()):
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
    # Every module is imported before anything is printed, so a module that is
    # slow to import cannot interleave with the listing itself.
    described = {name: describe(name) for name in candidates()}

    print(f"pentimento {version()}: build, audit, pack and verify the "
          f"Pentimento corpus\n")
    print("  pentimento <command> [options]")
    print("  pentimento <command> --help\n")

    runnable = {n: d for n, d in described.items() if isinstance(d, tuple)}
    for name, (_, summary) in sorted(runnable.items()):
        # Shortened on a word boundary with a visible ellipsis. A hard cut at a
        # fixed column lands mid-word and reads as corrupted output rather than
        # as a summary that did not fit.
        short = textwrap.shorten(summary, width=SUMMARY_WIDTH,
                                 placeholder=" ...")
        print(f"  {name.replace('_', '-'):28} {short}")

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
    with_defaults_in_help()

    if not argv or argv[0] in ("-h", "--help", "help", "list"):
        return listing()

    if argv[0] in ("-V", "--version", "version"):
        print(f"pentimento {version()}")
        return 0

    wanted = normalise(argv[0])
    if wanted not in candidates():
        guesses = nearest(wanted, candidates())
        print(f"no such command: {argv[0]}", file=sys.stderr)
        if guesses:
            print(f"did you mean: "
                  f"{', '.join(n.replace('_', '-') for n in guesses)}",
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
    warn_if_unlocked(wanted, argv[1:])
    # argparse names a program after argv[0], so every subcommand's usage line
    # read `usage: pentimento [-h] --covers ...` and could not be copied: what
    # the reader typed was `pentimento build-core-tier`.
    sys.argv[0] = f"pentimento {argv[0].replace('_', '-')}"
    return module.main(argv[1:])


if __name__ == "__main__":
    raise SystemExit(main())

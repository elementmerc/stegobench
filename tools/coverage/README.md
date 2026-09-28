# Coverage

This directory measures how much of the repository the tests actually reach,
and holds a floor so the answer cannot quietly get worse.

Before it existed, nothing here measured coverage at all. CLAUDE.md Section 7
asks for at least 90% branch coverage on changed files, and every claim of
meeting that was a count of tests written rather than a measurement.

## Running it

From the repository root, one command per half:

```sh
python3 tools/coverage/measure.py python
python3 tools/coverage/measure.py rust
```

Each one runs the real test suites, prints a table of every file below 90%,
prints the total for each component, and exits non-zero if a total has fallen
below the floor recorded in `floors.toml`.

What you need installed:

| Half | Needs | Install |
|---|---|---|
| Python | `coverage` 7.16.0 | `pip install coverage==7.16.0` |
| Rust | `cargo-llvm-cov` 0.8.7 and the `llvm-tools` rustup component | `cargo install cargo-llvm-cov --locked --version 0.8.7` then `rustup component add llvm-tools-preview` |

The script checks for both before it starts and tells you the install command
if one is missing, rather than failing halfway through a long run.

## The figures, and what they are figures of

Measured on 2026-09-28, on the committed tree, on a developer machine:

| Component | Coverage | Unit |
|---|---:|---|
| `generators` | 44.54% | statements and branches |
| `tools/release` | 68.05% | statements and branches |
| Rust workspace | 90.42% | regions |

The units differ and the difference matters.

**Python is branch coverage.** `coveragerc` sets `branch = True`, and the
figure above blends statements with branch outcomes. Line coverage on its own
overstates: a one-line `if` whose else is never taken counts as fully covered
by lines and half covered by branches, and the rule in CLAUDE.md is about
branches.

**Rust is region coverage, not branch coverage.** `cargo llvm-cov --branch`
needs a nightly compiler, and `rust-toolchain.toml` pins stable 1.98.1 on
purpose, because a floating toolchain turned an untouched branch red on
2026-09-18. Region coverage is the closest honest measure available on stable:
finer than line coverage, since an expression that never evaluates is its own
region, but it is not branch coverage and nothing here says it is. If the
toolchain ever moves to nightly, `--branch` becomes available and the unit
should change with it.

## Why the gate is a ratchet

A whole-tree gate set at 90% would have failed on its first run, on code
nobody is touching. That would break two rules at once: CLAUDE.md Section 6,
which leaves pre-existing problems on untouched code alone, and the lesson
this repository already learned with `docs.yml`, which is
`workflow_dispatch` only because a permanently red check costs more than it
reports.

A job that only prints a number, though, is decorative.

So `floors.toml` records the coverage already reached, per component. A run
below its floor fails; a run above it passes and says the floor can be
raised. The floor moves up only through

```sh
python3 tools/coverage/measure.py python --bump
```

which rewrites `floors.toml` for you to commit. CI never writes it. The file
is committed, so its history is the record of every move the floor has made,
and lowering one is a hand edit in its own commit with the reason in the
message.

## The 90% rule is reported, not enforced

Both runs list every file under 90% with its figure. That list is advisory.

Enforcing 90% per changed file would mean a one-line typo fix in a module
sitting at 6% turns a pull request red and demands a test-writing project
nobody asked for. That is the same failure as a permanently red check. The
division is that the machine holds the line and a human reads the list: if
your change touches a file on it, that is the row to fix before the work is
done.

## What is excluded, and why

Every exclusion is in `coveragerc` with the reason written next to it, because
an unexplained exclusion is how a coverage figure becomes decorative. In
summary:

| Excluded | Why |
|---|---|
| `*/test_*.py` | A suite that measures itself scores near 100% and drags the total up by the size of the suite |
| `*/__pycache__/*` | Build artefacts, not source |
| `tools/coverage/*` | The harness runs the tests rather than being run by them, so it would report on itself |
| `if TYPE_CHECKING:` | Never executes at runtime by design, and contains no behaviour |
| `if __name__ == "__main__":` | Unreachable from an imported test by construction; the `main()` it calls is covered |
| `raise NotImplementedError` | An abstract method whose contract is that it is never called |
| `pragma: no cover` | The standard escape hatch, kept so a genuinely unreachable line is marked where it lives |

Those exclusions cost 225 statements out of 8,141 in the Python half, 2.76% of
it, and nearly all of that is one `if __name__ == "__main__":` block per script
across sixty-odd scripts. Those blocks would have scored close to zero, so
excluding them lifts the reported figure by roughly two and a half points. That
is the whole of the inflation and it is written here rather than left for
somebody to find.

Three things are deliberately **not** excluded, because each would hide
something real: `except ImportError` blocks, which are the optional-dependency
degradation paths a reader without the optional set depends on; `sys.platform`
blocks, since this half claims three operating systems and CI runs it on
three; and defensive `raise` lines, which are the fail-loud error paths
CLAUDE.md Section 2 asks for and which rot the moment they stop being
measured.

On the Rust side nothing is excluded. Dependencies compiled from outside the
tree are dropped from the report because they are not ours to measure, and
that is the only filtering that happens.

## In CI

`.github/workflows/ci.yml` runs both halves as `Coverage, Python half` and
`Coverage, Rust workspace`, on Ubuntu, and writes the file table into the run
summary so a reviewer sees which files are below the bar rather than only a
percentage. The full reasoning for the threshold is in a comment above those
jobs.

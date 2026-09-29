#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Do the addresses in this repository's published prose still go anywhere?

WHY THIS IS SEPARATE FROM THE OTHER TWO CHECKERS
================================================

Three things here ask questions that sound alike and are not:

- `generators/verify_release.py --check-urls` reads the prose packed INSIDE a
  corpus release and asks whether its addresses answer. It needs the corpus, so
  it runs on the build box and cannot run in CI.
- `tools/release/verify_published.py` asks whether each destination we publish
  to is serving the release we think we sent it.
- This one asks whether the addresses in the prose THIS REPOSITORY ships still
  reach anything. A reader who clones or browses the repo follows these, and
  nothing was checking them on any schedule.

The failure this exists to catch has already happened once next door. A
`DATASHEET.md` told photographers to raise objections at a GitHub URL that
answered 404 for the whole of a usability walkthrough, and the licence the
corpus depends on assumes that channel exists. A dead address in prose is not a
cosmetic defect; it is a promise that turns out to be empty.

WHAT IT FAILS ON, AND WHAT IT ONLY REPORTS
==========================================

Only a definite refusal (a 4xx) fails the run. A host that times out, refuses a
connection or answers 429 or 5xx has told us about its own weather rather than
about our prose, and failing on that would produce a red tick that is right
often enough to be ignored. Those are reported by name and counted, never
folded into the clean line, because "everything answers" and "everything we
managed to ask answers" are different sentences.

WHAT IT SKIPS ON PURPOSE
========================

Documentation contains addresses that are meant to be unreachable: the example
endpoints in the guides, the private addresses used to show a refusal, and the
RFC 2606 example domains. Checking those would produce a permanent failure that
teaches everybody to ignore the job. They are skipped by rule rather than by a
hand-maintained list of exceptions, and the rules are named in the output so a
reader can see what was not asked.
"""

from __future__ import annotations

import argparse
import datetime as dt
import ipaddress
import json
import pathlib
import re
import subprocess
import sys
import time
import urllib.parse

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python 3.10 and earlier
    import tomli as tomllib  # type: ignore[no-redef]

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from verify_published import fetch  # noqa: E402  the one bounded HTTP client here

#: Files whose addresses a reader actually follows. Deliberately a list of
#: globs rather than "every tracked file": a URL in a test fixture or a source
#: comment is not a promise to anybody, and checking it would spend the budget
#: on addresses nobody visits.
PROSE_GLOBS = (
    "README.md",
    "llms.txt",
    "CHANGELOG.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "CODE_OF_CONDUCT.md",
    "CITATION.cff",
    "docs/**/*.md",
    "plugins/registry/**/*.toml",
)

#: The whole run's budget. A nightly job that can run long enough to matter is
#: one somebody cancels. When it expires the addresses not yet reached are
#: reported as unreached rather than counted as fine.
DEFAULT_BUDGET_SECONDS = 240.0

#: The most addresses to ask about in one run, spread across hosts so a single
#: busy host cannot consume the whole sample. Raised with --cap.
DEFAULT_CAP = 200

#: A definite refusal. Everything else the host says is about the host.
DEAD_STATUSES = frozenset({400, 401, 403, 404, 405, 410, 451})

#: Reserved for documentation by RFC 2606 and RFC 6761. They are guaranteed not
#: to be real, which is exactly why prose uses them.
EXAMPLE_DOMAINS = ("example.com", "example.org", "example.net", "example.edu",
                   "example", "invalid", "test", "localhost")

#: A backslash ends the address. Prose here carries both escaped regexes (the
#: OIDC subject in RELEASING.md is written `https://github\.com/...`) and TOML
#: line continuations, and in both the backslash belongs to the surrounding
#: text. Including it produced two requests for addresses nobody could visit.
URL_PATTERN = re.compile(r"https?://[^\s<>\"')\]}`,\\]+")

#: Trailing punctuation that belongs to the sentence rather than to the address.
TRAILING = ".,;:!?'\"\\"


def tidy(url: str) -> str:
    """Strip the sentence punctuation a URL picks up from prose around it."""
    while url and url[-1] in TRAILING:
        url = url[:-1]
    # A closing bracket is only part of the address when an opening one is too,
    # which is how Wikipedia-shaped URLs survive being written in a sentence.
    while url.endswith(")") and url.count("(") < url.count(")"):
        url = url[:-1]
    return url


def why_skipped(url: str) -> str | None:
    """Say why this address is not worth asking about, or None to ask."""
    try:
        parsed = urllib.parse.urlparse(url)
    except ValueError as exc:
        return f"is not a URL this can parse ({exc})"
    host = (parsed.hostname or "").lower()
    if not host:
        return "names no host"
    if "<" in url or ">" in url or "{" in url:
        return "is a template rather than an address"
    try:
        address = ipaddress.ip_address(host)
    except ValueError:
        address = None
    if address is not None:
        if (address.is_loopback or address.is_private or address.is_link_local
                or address.is_unspecified):
            # The guides print these deliberately, to show a reader the refusal
            # they get for one. Asking would fail on every run.
            return "is a private or loopback address, printed as an example"
        return None
    if host in EXAMPLE_DOMAINS or any(
            host.endswith("." + d) for d in EXAMPLE_DOMAINS):
        return "is a reserved example domain"
    if "." not in host:
        return "is not a resolvable name, so it is an example"
    return None


#: Directories that match the prose globs and are not this project's prose.
#: The first run of this checker read `docs/node_modules` and reported eight
#: dead addresses belonging to other people's packages, which is worse than
#: useless: it is a red tick for somebody else's broken link.
NOT_OURS = ("node_modules", ".vitepress/cache", ".vitepress/dist", "target")


def prose_files(root: pathlib.Path) -> list[pathlib.Path]:
    """The prose files that actually ship, preferring what git tracks.

    Tracked-only is the honest definition. A file git ignores is not published,
    and the vendored packages under `docs/node_modules` match the same globs
    while belonging to other people. Where git cannot answer (a tarball rather
    than a clone), the globs are used with the vendored directories excluded by
    name, which is coarser and is why git is asked first.
    """
    paths: list[pathlib.Path] = []
    try:
        listed = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z", "--", *PROSE_GLOBS],
            capture_output=True, timeout=30, check=True,
        )
        for name in listed.stdout.decode("utf-8", "replace").split("\0"):
            if name:
                paths.append(root / name)
    except (OSError, subprocess.SubprocessError) as exc:
        print(f"git could not list the prose ({exc}); falling back to the "
              f"globs, which is coarser", file=sys.stderr)
        for pattern in PROSE_GLOBS:
            paths.extend(root.glob(pattern))
        paths = [
            p for p in paths
            if not any(part in p.relative_to(root).as_posix() for part in NOT_OURS)
        ]
    # Ordered by the posix spelling rather than by the path object, because
    # path comparison follows the platform: Windows folds case, so it sorts
    # `docs\guide\x.md` before `README.md` while every other machine does
    # the reverse. That order reaches the report, and a report whose row
    # order depends on the OS that wrote it is not reproducible.
    return sorted({p for p in paths if p.is_file()},
                  key=lambda p: p.relative_to(root).as_posix())


def addresses_in(root: pathlib.Path) -> dict[str, list[str]]:
    """Every address in the published prose, mapped to the files naming it."""
    found: dict[str, list[str]] = {}
    for path in prose_files(root):
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as exc:
            print(f"could not read {path}: {exc}", file=sys.stderr)
            continue
        for raw in URL_PATTERN.findall(text):
            url = tidy(raw)
            if not url:
                continue
            where = path.relative_to(root).as_posix()
            names = found.setdefault(url, [])
            if where not in names:
                names.append(where)
    return found


def spread(urls: list[str], cap: int) -> list[str]:
    """Take up to `cap` addresses, one host at a time, so no host dominates.

    A flat truncation would spend the whole sample on whichever host happens to
    be named most often, which here is GitHub, and prove nothing about the
    licence URLs that matter most.
    """
    by_host: dict[str, list[str]] = {}
    for url in urls:
        host = (urllib.parse.urlparse(url).hostname or "").lower()
        by_host.setdefault(host, []).append(url)
    for bucket in by_host.values():
        bucket.sort()
    chosen: list[str] = []
    hosts = sorted(by_host)
    while len(chosen) < cap and any(by_host[h] for h in hosts):
        for host in hosts:
            if by_host[host] and len(chosen) < cap:
                chosen.append(by_host[host].pop(0))
    return chosen


def look(url: str, fetcher=None) -> dict:
    """Ask one address. Returns a row, never raises."""
    fetcher = fetcher or fetch
    row = {"url": url, "status": None, "verdict": "unknown", "detail": None}
    try:
        got = fetcher(url)
    except Exception as exc:  # the fetcher has already retried what it should
        row["verdict"] = "unreachable"
        row["detail"] = str(exc)
        return row
    row["status"] = got.status
    if got.status in DEAD_STATUSES:
        row["verdict"] = "dead"
        row["detail"] = f"answered {got.status}"
    elif got.status >= 400:
        # 429 and the 5xx range: the host declining to talk this minute. Saying
        # so is honest; failing on it would be a red tick nobody trusts.
        row["verdict"] = "unsettled"
        row["detail"] = f"answered {got.status}, which is about the host"
    else:
        row["verdict"] = "alive"
    return row


def load_known(path: pathlib.Path) -> dict[str, str]:
    """Addresses known not to answer, mapped to why. Missing file means none."""
    if not path.exists():
        return {}
    with path.open("rb") as handle:
        parsed = tomllib.load(handle)
    known: dict[str, str] = {}
    for entry in parsed.get("address", []):
        url = entry.get("url", "").strip()
        reason = " ".join(entry.get("reason", "").split())
        if not url or not reason:
            raise SystemExit(
                f"{path}: every entry needs a url and a reason a stranger can "
                f"evaluate. An exemption nobody can justify gets widened by "
                f"whoever trips over it next"
            )
        if url in known:
            raise SystemExit(f"{path}: {url} is listed twice")
        known[url] = reason
    return known


def main(argv: list[str] | None = None) -> int:
    here = pathlib.Path(__file__).resolve().parents[2]
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--root", type=pathlib.Path, default=here,
                    help="the repository whose prose to read")
    ap.add_argument("--cap", type=int, default=DEFAULT_CAP)
    ap.add_argument("--budget", type=float, default=DEFAULT_BUDGET_SECONDS)
    ap.add_argument("--out", type=pathlib.Path,
                    help="where to write the record; omit to print it")
    ap.add_argument("--known", type=pathlib.Path,
                    default=pathlib.Path(__file__).resolve().parent
                    / "prose-links-known.toml",
                    help="addresses known not to answer yet, and why")
    args = ap.parse_args(argv)

    if args.cap < 1:
        print("--cap must be at least 1", file=sys.stderr)
        return 2
    if args.budget <= 0:
        print("--budget must be positive", file=sys.stderr)
        return 2

    found = addresses_in(args.root)
    if not found:
        # Refused rather than passed. A run that read no prose and exited zero
        # is the same shape of lie the post-publish verifier refuses next door.
        print(f"no addresses found under {args.root}, so this checked nothing",
              file=sys.stderr)
        return 2

    known = load_known(args.known)
    skipped = {u: why for u in found if (why := why_skipped(u))}
    checkable = sorted(u for u in found if u not in skipped)
    chosen = spread(checkable, args.cap)

    rows: list[dict] = []
    unreached = 0
    started = time.monotonic()
    for index, url in enumerate(chosen):
        if time.monotonic() - started > args.budget:
            unreached = len(chosen) - index
            break
        row = look(url)
        row["named_in"] = found[url]
        if url in known:
            row["known_reason"] = known[url]
            if row["verdict"] == "dead":
                row["verdict"] = "expected"
            elif row["verdict"] == "alive":
                # The reason this was written down has gone. Left alone, the
                # entry would go on excusing a genuinely dead address later.
                row["verdict"] = "no-longer-expected"
        rows.append(row)
        mark = {"alive": "ok", "dead": "DEAD", "unreachable": "no answer",
                "unsettled": "not proven", "expected": "not yet",
                "no-longer-expected": "STALE"}[row["verdict"]]
        print(f"{mark:<10} {url}", flush=True)
        if row["verdict"] != "alive":
            detail = row["detail"] or row.get("known_reason") or "answers now"
            print(f"           {detail}  (in {', '.join(row['named_in'])})",
                  flush=True)

    dead = [r for r in rows if r["verdict"] == "dead"]
    expected = [r for r in rows if r["verdict"] == "expected"]
    expired = [r for r in rows if r["verdict"] == "no-longer-expected"]
    unreachable = [r for r in rows if r["verdict"] == "unreachable"]
    unsettled = [r for r in rows if r["verdict"] == "unsettled"]
    alive = [r for r in rows if r["verdict"] == "alive"]

    document = {
        "checked_utc": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "root": str(args.root),
        "distinct_addresses": len(found),
        "skipped": skipped,
        "known_not_live": known,
        "unreached": unreached,
        "results": sorted(rows, key=lambda r: r["url"]),
    }
    text = json.dumps(document, indent=2, sort_keys=True) + "\n"
    if args.out:
        args.out.write_text(text, encoding="utf-8")
        print(f"wrote {args.out}")
    else:
        print(text)

    print(f"{len(found)} distinct address(es) in the published prose: "
          f"{len(skipped)} skipped as examples, {len(alive)} answer, "
          f"{len(dead)} dead, {len(expected)} not live yet by prior "
          f"agreement, {len(expired)} listed as not live and answering now, "
          f"{len(unreachable)} gave no answer, {len(unsettled)} not proven "
          f"either way, {unreached} not reached before the "
          f"{args.budget:.0f}s budget ran out.")

    if unreachable:
        print("An address that gave no answer at all is NOT the same as a 404. "
              "The page may be alive and this machine off the network, so "
              "confirm connectivity before believing it.", file=sys.stderr)
    if expired:
        # The entry outlived its reason. Left alone it would go on excusing an
        # address that genuinely breaks later, which is how an exemption list
        # turns into a place things hide.
        print("These addresses are listed in the known-not-live file and are "
              "answering now, so the reason they were written down has gone. "
              "Delete their entries: " + ", ".join(r["url"] for r in expired),
              file=sys.stderr)
    if dead:
        print("A dead address in published prose is a promise that turns out "
              "to be empty. The objection channel a licence depends on is the "
              "worst of them to lose.", file=sys.stderr)
    if dead or expired:
        return 1
    if not alive and not expected:
        # Every host declined, so this run looked at addresses and proved
        # nothing about any of them. Exiting zero would read as a clean bill.
        # An `expected` row counts here: a 404 we predicted is a real answer
        # from a reachable host, which is exactly what this guard is asking
        # about.
        print("Nothing answered, so this run proves nothing about any address.",
              file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

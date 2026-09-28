#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Check that a release actually reached the pages people load.

WHY THIS IS A SEPARATE STEP FROM PUBLISHING
===========================================

An upload that returns success says nothing about what the public page shows.
This project has been bitten twice by exactly that: an Internet Archive page
carried a wrong cover count for four days, and a corrected Kaggle description
sat on disk while the live page served the old one. Both were "published" by a
step that exited zero, and both were found by a person happening to look.

`generators/verify_release.py` decides whether a packed release is fit to leave
this machine. This one asks the opposite question, from outside: does every
destination now serve what we think we sent it? They are different questions
and neither substitutes for the other.

WHY IT ALSO RUNS ON A SCHEDULE
==============================

A channel can go wrong long after the release. A dataset gets taken down, a
mirror expires, a docs build starts failing silently, a crate yanks. Running
this nightly turns "somebody noticed eventually" into "we knew within a day".

WHAT IT REFUSES TO DO
=====================

It does not fix anything and it does not publish anything. It reads public
pages and reports. A verifier that repairs what it finds is a publisher with a
misleading name, and the whole value here is that it has no stake in the
answer.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import pathlib
import sys
import time
import urllib.error
import urllib.request

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python 3.10 and earlier
    import tomli as tomllib  # type: ignore[no-redef]


#: How long any single request may take. Baseline Section 2.1: no blocking
#: network call waits forever. A page that needs longer than this is a page a
#: reader would give up on anyway.
TIMEOUT_SECONDS = 20

#: The most of a response body this will read. Every check here looks at a page
#: or a small JSON document; a destination that answers with a gigabyte is
#: either broken or hostile, and reading it would be the verifier's own denial
#: of service.
MAX_BODY_BYTES = 4 * 1024 * 1024

#: Statuses worth trying again. Everything else is an answer: a 404 means the
#: page is not there, and asking four more times will not change that. This
#: distinction is written down because the first version of this project's
#: upload retry helper retried every failure, including its own refusals, and
#: spent two minutes sleeping before reporting a permanent error.
RETRYABLE_STATUSES = frozenset({429, 500, 502, 503, 504})

ATTEMPTS = 3
BACKOFF_SECONDS = 2.0

#: A user agent that says who is asking. Several of these hosts rate-limit an
#: unidentified client harder, and an operator reading their own logs should be
#: able to tell this apart from a scraper.
USER_AGENT = "stegobench-release-verifier (+https://github.com/elementmerc/stegobench)"


class Fetched:
    """One response, reduced to what a check needs."""

    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.body = body


def fetch(url: str) -> Fetched:
    """GET a URL, with a deadline, a bounded read and bounded retries."""
    last: Exception | None = None
    for attempt in range(1, ATTEMPTS + 1):
        request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
                raw = response.read(MAX_BODY_BYTES)
                return Fetched(response.status, raw.decode("utf-8", "replace"))
        except urllib.error.HTTPError as exc:
            body = ""
            try:
                body = exc.read(MAX_BODY_BYTES).decode("utf-8", "replace")
            except Exception:  # pragma: no cover - the body is a nicety
                pass
            if exc.code not in RETRYABLE_STATUSES:
                return Fetched(exc.code, body)
            last = exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            last = exc
        if attempt < ATTEMPTS:
            time.sleep(BACKOFF_SECONDS * attempt)
    raise RuntimeError(f"{url}: {last}")


def dig(document: object, path: str) -> object:
    """Follow a dotted path into parsed JSON, or raise saying where it stopped."""
    here: object = document
    walked: list[str] = []
    for key in path.split("."):
        if not isinstance(here, dict) or key not in here:
            place = ".".join(walked) or "the top level"
            raise KeyError(f"no {key!r} at {place}")
        here = here[key]
        walked.append(key)
    return here


def check(channel: dict, version: str, fetcher=None) -> dict:
    """Check one channel. Returns a row for distribution.json, never raises.

    `fetcher` is resolved at call time rather than bound as a default, so a
    test can replace `fetch` on the module and have it take effect here. Bound
    as a default it did not, and three tests quietly made real network calls
    while appearing to use a stub: two of them passed for the wrong reason and
    the suite took eighteen seconds.
    """
    fetcher = fetcher or fetch
    url = channel["url"].replace("{version}", version)
    row = {
        "id": channel["id"],
        "url": url,
        "required": bool(channel.get("required", True)),
        "ok": False,
        "problem": None,
        "status": None,
        # A channel that is live and still optional is a channel somebody
        # published to and forgot to make load-bearing. See `main`.
        "should_be_required": False,
    }
    try:
        got = fetcher(url)
    except Exception as exc:  # the fetcher has already retried what it should
        row["problem"] = f"could not be reached: {exc}"
        return row

    row["status"] = got.status
    if got.status != 200:
        row["problem"] = f"answered {got.status}"
        return row

    if "json_path" in channel:
        try:
            document = json.loads(got.body)
        except json.JSONDecodeError as exc:
            row["problem"] = f"did not answer with JSON: {exc}"
            return row
        try:
            found = dig(document, channel["json_path"])
        except KeyError as exc:
            row["problem"] = f"{channel['json_path']} is missing: {exc}"
            return row
        want = channel["must_equal"].replace("{version}", version)
        row["found"] = found
        if str(found) != want:
            row["problem"] = f"{channel['json_path']} is {found!r}, expected {want!r}"
            return row

    missing = [
        needle
        for raw in channel.get("must_contain", [])
        for needle in [raw.replace("{version}", version)]
        if needle not in got.body
    ]
    if missing:
        # Named rather than counted. "2 strings missing" sends somebody to read
        # a page by hand; naming them is usually the whole diagnosis.
        row["problem"] = "the page does not mention " + ", ".join(repr(m) for m in missing)
        return row

    row["ok"] = True
    row["should_be_required"] = not row["required"]
    return row


def load_channels(path: pathlib.Path) -> list[dict]:
    with path.open("rb") as handle:
        parsed = tomllib.load(handle)
    channels = parsed.get("channel", [])
    if not channels:
        raise SystemExit(
            f"{path} declares no channels, so this would have checked nothing "
            f"and exited zero. That is the failure this tool exists to catch, "
            f"so it is refused here too."
        )
    seen: set[str] = set()
    for channel in channels:
        for field in ("id", "url"):
            if not channel.get(field):
                raise SystemExit(f"{path}: a channel has no {field}")
        if channel["id"] in seen:
            raise SystemExit(f"{path}: two channels share the id {channel['id']!r}")
        seen.add(channel["id"])
        if "json_path" in channel and "must_equal" not in channel:
            raise SystemExit(
                f"{path}: {channel['id']} names a json_path with nothing to "
                f"compare it against, so it would pass on any value"
            )
        if "json_path" not in channel and not channel.get("must_contain"):
            raise SystemExit(
                f"{path}: {channel['id']} states no condition, so it would "
                f"report success for any page that answers 200, including a "
                f"parked domain"
            )
    return channels


def main(argv: list[str] | None = None) -> int:
    here = pathlib.Path(__file__).resolve().parent
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--version", required=True, help="the release version, without the v")
    ap.add_argument("--channels", type=pathlib.Path, default=here / "channels.toml")
    ap.add_argument(
        "--out",
        type=pathlib.Path,
        help="where to write distribution.json; omit to print it",
    )
    ap.add_argument(
        "--only",
        default="",
        help="check one channel by id, for re-checking a single destination",
    )
    args = ap.parse_args(argv)

    channels = load_channels(args.channels)
    if args.only:
        channels = [c for c in channels if c["id"] == args.only]
        if not channels:
            print(f"no channel with id {args.only!r}", file=sys.stderr)
            return 2

    rows = []
    for channel in channels:
        row = check(channel, args.version)
        rows.append(row)
        if row["should_be_required"]:
            mark = "FLIP"
        elif row["ok"]:
            mark = "ok"
        elif row["required"]:
            mark = "MISS"
        else:
            mark = "not yet"
        print(f"{mark:<8} {row['id']:<28} {row['url']}", flush=True)
        if row["problem"]:
            print(f"         {row['problem']}", flush=True)

    document = {
        "version": args.version,
        "checked_utc": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "channels": rows,
    }
    text = json.dumps(document, indent=2, sort_keys=True) + "\n"
    if args.out:
        # Written whatever the verdict. A record of a failed check is more
        # useful than no record, and the exit code carries the verdict.
        args.out.write_text(text, encoding="utf-8")
        print(f"wrote {args.out}")
    else:
        print(text)

    failed = [r for r in rows if r["required"] and not r["ok"]]
    waiting = [r for r in rows if not r["required"] and not r["ok"]]
    flip = [r for r in rows if r["should_be_required"]]
    print(
        f"{len(rows)} channel(s): {sum(1 for r in rows if r['ok'])} serving "
        f"{args.version}, {len(failed)} wrong, {len(waiting)} not live yet, "
        f"{len(flip)} live but still optional."
    )
    if failed:
        print(
            "A destination that is required and wrong is a public page saying "
            "something untrue about this release.",
            file=sys.stderr,
        )
    if flip:
        # `required = false` exists so this file can describe a channel before
        # it is live. Once the page answers with the release, that reason is
        # spent, and leaving it optional means a later takedown of a channel
        # people are already using would be reported and pass. Flipping it is
        # a step in RELEASING.md, which is to say it depended on somebody
        # remembering, which is the failure this whole tool exists to remove.
        print(
            "These channels are serving the release and are still marked "
            "`required = false` in channels.toml, so a later takedown would "
            "be reported and pass. Set `required = true` for: "
            + ", ".join(r["id"] for r in flip),
            file=sys.stderr,
        )
    if failed or flip:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

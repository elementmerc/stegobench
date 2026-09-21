#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Find out WHY each "Public domain" cover is public domain, and record it.

WHY THIS EXISTS
---------------
This corpus's reason to exist is that its licences are traceable, in a field
where BOSSbase and ALASKA2 cannot be republished at all and where mirrors of
Dresden and UCID carry licences their sources never granted.

Against that claim, "Public domain" is the one entry in the licence table that
explains nothing. Every other licence in the corpus is a named CC instrument
with a URL a reader can open and read. Roughly a fifth of the covers instead
carry a bare string, no URL, and no statement of the ground being relied on.

The grounds are not interchangeable. A NASA photograph is uncopyrighted because
it is a work of the US federal government. A painting from the 1890s is
uncopyrighted because its author has been dead long enough, which is a fact
about a jurisdiction and a date rather than about the file. A photograph of a
public domain painting is a contested case in some jurisdictions and settled in
others. A reader in Berlin and a reader in Ohio can reach different answers
about the same file, and only the specific ground lets either of them work it
out.

Commons records the ground as a template on the file page. `extmetadata` does
not carry it: there, everything collapses to `License: pd`, which is how the
distinction was lost on the way into the manifest in the first place.

WHAT IT DOES
------------
Reads the cover manifest, takes the rows recorded as public domain, asks
Commons which templates each file page uses, and keeps the ones that state a
copyright ground. Writes an augmented manifest and prints the distribution.

It is read-only with respect to the corpus: nothing is re-downloaded and no
image is touched. The output is a new manifest beside the old one, so a bad run
costs nothing.

Usage::

    python audit_pd_licences.py --manifest ~/pentimento/covers/commons/manifest.jsonl
    python audit_pd_licences.py --manifest ... --write   # write the augmented copy
"""
from __future__ import annotations

import argparse
import collections
import json
import pathlib
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

#: Commons answers up to 50 titles per query for anonymous callers.
BATCH = 50

#: Named so a Commons administrator can tell who is calling and stop us if we
#: are being a nuisance. An anonymous script hammering the API is how a project
#: gets blocked for everybody.
USER_AGENT = ("pentimento-licence-audit/1.0 "
              "(+https://github.com/elementmerc/stegobench; "
              "daniel@themalwarefiles.com)")

API = "https://commons.wikimedia.org/w/api.php"

#: Between requests. Commons asks for serial access rather than a fixed rate,
#: and 40 requests taking a minute is not worth optimising.
COURTESY_DELAY = 0.5

#: A template states a copyright ground if its name starts with one of these.
#: `PD-` covers the overwhelming majority; the others are the common aliases
#: that do not carry the prefix.
GROUND_PREFIXES = ("PD-", "PD ", "Public domain")
GROUND_EXACT = {
    "Template:PD", "Template:CC0", "Template:No rights reserved",
    "Template:Copyrighted free use", "Template:Attribution",
}

#: Templates that carry the PD prefix and state nothing. They are the shared
#: furniture a licence tag is rendered with.
#:
#: `PD-user-w` is NOT in here, though it looks like furniture. It is a real
#: ground: a named contributor releasing their own work. Excluding it put 20
#: covers in the "no stated ground" pile, where they read as a licensing
#: problem rather than as one of the better documented cases in the corpus.
PRESENTATION = {"PD-Layout", "PD-Art/layout"}

#: Licence families we might see on a file page, for the cross-check. The point
#: is to notice when the page says something the manifest does not.
LICENCE_PREFIXES = ("PD", "Cc-", "CC-", "GFDL", "FAL", "Attribution",
                    "Copyrighted free use", "No rights reserved")


class AuditError(RuntimeError):
    """Raised when Commons cannot be asked or cannot be understood."""


def ground_templates(titles: list[str]) -> dict[str, list[str]]:
    """Map each file title to the templates on its page that state a ground.

    Raises rather than returning partial results. A missing answer here would
    become a cover recorded as having no stated ground, which is exactly the
    gap this tool exists to close, and it would look like a finding rather than
    a network problem.

    CONTINUATION IS NOT OPTIONAL HERE. `tllimit` bounds the templates returned
    across the WHOLE query, not per page, so a batch of 50 files exhausts it
    part way through and every page after that point comes back with no
    templates at all. Read without continuing, this tool reported two in five
    covers as having no stated public domain ground, and every one of them
    turned out to say PD-USGov-NASA or similar on the page itself. A partial
    answer that looks like a complete one is the worst shape a bug can take in
    a licence audit.
    """
    found: dict[str, list[str]] = {t: [] for t in titles}
    params = {
        "action": "query",
        "titles": "|".join(titles),
        "prop": "templates",
        "tlnamespace": "10",
        "tllimit": "max",
        "format": "json",
        "formatversion": "2",
    }
    while True:
        request = urllib.request.Request(
            f"{API}?{urllib.parse.urlencode(params)}",
            headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                payload = json.load(response)
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as e:
            raise AuditError(f"Commons did not answer: {e}") from e

        if "query" not in payload:
            raise AuditError(
                f"unexpected answer from Commons: {str(payload)[:200]}")

        for page in payload["query"].get("pages", []):
            grounds = [
                t["title"] for t in page.get("templates", [])
                if is_ground(t["title"])
            ]
            found.setdefault(page["title"], [])
            found[page["title"]].extend(grounds)

        if "continue" not in payload:
            break
        params.update(payload["continue"])
        time.sleep(COURTESY_DELAY)

    return {title: sorted(set(g)) for title, g in found.items()}


def is_ground(template: str) -> bool:
    """Whether a template states a copyright ground rather than dressing one.

    Commons builds a licence tag out of several templates: the tag itself, plus
    a `/layout`, a `/en`, a `/core` and often a shared `PD-Layout`. Those are
    presentation. Counting them produces a table where the commonest "ground"
    is PD-Layout, which tells a reader nothing at all.
    """
    if template in GROUND_EXACT:
        return True
    name = template.split(":", 1)[-1]
    if name in PRESENTATION or "/" in name:
        return False
    return any(name.startswith(p) for p in GROUND_PREFIXES)


def shorten(template: str) -> str:
    """`Template:PD-USGov-NASA` to `PD-USGov-NASA`."""
    return template.split(":", 1)[-1]


def licence_templates(titles: list[str]) -> dict[str, list[str]]:
    """Every licence-family template on each page, not only the PD ones.

    Used to cross-check the recorded licence against what the file page says.
    A file recorded as public domain whose page carries `Cc-by-sa-4.0` is not a
    formatting quirk: share-alike is a materially stricter obligation than
    anything the corpus claims to contain, and publishing it as public domain
    tells every downstream user they may ignore terms that actually bind them.
    """
    found: dict[str, list[str]] = {t: [] for t in titles}
    params = {
        "action": "query",
        "titles": "|".join(titles),
        "prop": "templates",
        "tlnamespace": "10",
        "tllimit": "max",
        "format": "json",
        "formatversion": "2",
    }
    while True:
        request = urllib.request.Request(
            f"{API}?{urllib.parse.urlencode(params)}",
            headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                payload = json.load(response)
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as e:
            raise AuditError(f"Commons did not answer: {e}") from e
        if "query" not in payload:
            raise AuditError(f"unexpected answer: {str(payload)[:200]}")
        for page in payload["query"].get("pages", []):
            names = [shorten(t["title"]) for t in page.get("templates", [])]
            found.setdefault(page["title"], [])
            found[page["title"]].extend(
                n for n in names
                if "/" not in n and n not in PRESENTATION
                and not n.endswith("-layout")
                and any(n.startswith(p) for p in LICENCE_PREFIXES))
        if "continue" not in payload:
            break
        params.update(payload["continue"])
        time.sleep(COURTESY_DELAY)
    return {title: sorted(set(g)) for title, g in found.items()}


def family(template: str) -> str:
    """The obligation family a licence template belongs to.

    Coarse on purpose. The question being asked is not "which exact version"
    but "does the page impose something the manifest does not admit to", and
    share-alike against public domain is the answer that matters.
    """
    lowered = template.lower()
    if lowered.startswith(("cc-by-sa", "cc-sa")) or lowered.startswith("gfdl"):
        return "share-alike"
    # `Cc-pd` and `Cc-zero` are public domain dedications that happen to carry
    # the CC prefix. Reading the prefix alone files them as attribution
    # licences and reports a disagreement that is not one.
    if lowered.startswith(("cc-zero", "cc-pd")) or lowered == "cc0":
        return "public domain"
    if lowered.startswith("cc-by"):
        return "attribution"
    if lowered.startswith(("pd", "no rights reserved", "copyrighted free use")):
        return "public domain"
    if lowered.startswith("attribution"):
        return "attribution"
    return "other"


def cross_check(manifest: pathlib.Path, limit: int = 0) -> int:
    """Compare every recorded licence with what its Commons page states.

    The corpus's claim is that its licences are traceable. This is the check
    that the claim survives contact with the source: not that a licence string
    is present, but that it is the licence the file actually carries.
    """
    if not manifest.is_file():
        print(f"no manifest at {manifest}", file=sys.stderr)
        return 1
    rows = [json.loads(line) for line in manifest.read_text().splitlines()
            if line.strip()]
    if limit:
        rows = rows[:limit]
    print(f"cross-checking {len(rows):,} rows against Commons")

    by_title: dict[str, list[str]] = {}
    batches = [rows[i: i + BATCH] for i in range(0, len(rows), BATCH)]
    for n, batch in enumerate(batches, 1):
        try:
            by_title.update(licence_templates([r["title"] for r in batch]))
        except AuditError as e:
            print(f"\nbatch {n}/{len(batches)} failed: {e}", file=sys.stderr)
            return 1
        print(f"  batch {n}/{len(batches)}", end="\r")
        if n < len(batches):
            time.sleep(COURTESY_DELAY)
    print()

    disagreements: list[tuple[dict, list[str], set[str]]] = []
    silent: list[dict] = []
    for row in rows:
        templates = by_title.get(row["title"], [])
        if not templates:
            silent.append(row)
            continue
        families = {family(t) for t in templates}
        recorded = family(row.get("licence", "").replace(" ", "-"))
        if recorded == "other":
            recorded = ("public domain" if "public domain"
                        in row.get("licence", "").lower() else "attribution")
        # A page offering several licences is the uploader's choice to make and
        # ours to take: if ANY family on the page matches what we recorded, the
        # record is defensible. Only a page that offers nothing we claimed is a
        # disagreement.
        if recorded not in families:
            disagreements.append((row, templates, families))

    print(f"\n=== {len(disagreements):,} disagreement(s) ===")
    for row, templates, families in disagreements[:40]:
        print(f"  {row['file']}  recorded {row.get('licence')!r}")
        print(f"      page says: {', '.join(templates)}")
        print(f"      {row.get('descriptionurl', '')}")
    if len(disagreements) > 40:
        print(f"  ... and {len(disagreements) - 40:,} more")

    share_alike = [d for d in disagreements if "share-alike" in d[2]]
    if share_alike:
        print(f"\n*** {len(share_alike):,} carry a SHARE-ALIKE template. "
              f"Share-alike is stricter than anything this corpus claims to "
              f"contain, so the collection licence statement is wrong while "
              f"these are in it. ***")

    if silent:
        print(f"\n{len(silent):,} page(s) carry no licence template this tool "
              f"recognises. Not necessarily wrong; not verifiable either.")
        for row in silent[:10]:
            print(f"  {row['file']}  {row.get('descriptionurl', '')}")

    if not disagreements and not silent:
        print("\nEvery recorded licence matches a licence its page states.")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--licence", default="Public domain",
                    help="the licence string to audit")
    ap.add_argument("--write", action="store_true",
                    help="write the augmented manifest beside the original")
    ap.add_argument("--limit", type=int, default=0,
                    help="audit only the first n, for a dry run")
    ap.add_argument("--cross-check", action="store_true",
                    help="check EVERY row's recorded licence against the "
                         "licence templates on its Commons page, and report "
                         "where the two disagree")
    args = ap.parse_args(argv)

    if args.cross_check:
        return cross_check(pathlib.Path(args.manifest), args.limit)

    sys.stdout.reconfigure(line_buffering=True)
    manifest = pathlib.Path(args.manifest)
    if not manifest.is_file():
        print(f"no manifest at {manifest}", file=sys.stderr)
        return 1

    rows = [json.loads(line) for line in manifest.read_text().splitlines()
            if line.strip()]
    targets = [r for r in rows if r.get("licence") == args.licence]
    if args.limit:
        targets = targets[: args.limit]
    if not targets:
        print(f"no rows with licence {args.licence!r}", file=sys.stderr)
        return 1
    print(f"{len(targets):,} of {len(rows):,} rows to audit")

    by_title: dict[str, list[str]] = {}
    batches = [targets[i: i + BATCH] for i in range(0, len(targets), BATCH)]
    for n, batch in enumerate(batches, 1):
        try:
            by_title.update(ground_templates([r["title"] for r in batch]))
        except AuditError as e:
            print(f"\nbatch {n}/{len(batches)} failed: {e}", file=sys.stderr)
            return 1
        print(f"  batch {n}/{len(batches)}", end="\r")
        if n < len(batches):
            time.sleep(COURTESY_DELAY)
    print()

    counts: collections.Counter[str] = collections.Counter()
    ungrounded = []
    for row in targets:
        grounds = [shorten(t) for t in by_title.get(row["title"], [])]
        row["pd_grounds"] = grounds
        if grounds:
            counts.update(grounds)
        else:
            ungrounded.append(row["file"])

    print(f"\n=== grounds stated on {len(targets) - len(ungrounded):,} of "
          f"{len(targets):,} ===")
    for template, count in counts.most_common(40):
        print(f"{count:6,}  {template}")

    if ungrounded:
        print(f"\n{len(ungrounded):,} file(s) state NO ground. These are the "
              f"ones to look at by hand:")
        for name in ungrounded[:20]:
            print(f"  {name}")
        if len(ungrounded) > 20:
            print(f"  ... and {len(ungrounded) - 20:,} more")

    if args.write:
        out = manifest.with_suffix(".pd-audited.jsonl")
        augmented = {r["file"]: r for r in targets}
        out.write_text("".join(
            json.dumps(augmented.get(r["file"], r)) + "\n" for r in rows))
        print(f"\nwritten: {out}")
    else:
        print("\nnothing written. Pass --write for the augmented manifest.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

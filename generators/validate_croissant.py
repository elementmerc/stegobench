#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Check a Croissant record before it is published, with or without the
reference validator.

WHY THIS EXISTS
---------------
The Croissant record is the file that decides whether a dataset loads for
somebody who has never met us: Hugging Face, Kaggle and the ML Commons tooling
all read it, and a record that does not validate is a dataset that quietly does
not appear.

An earlier version of our record did not validate at all. It carried a
three-entry `@context` where the specification expects the full term map, had no
`recordSet` whatsoever, and its cover `FileSet` glob swallowed 769 arm shards
into the cover set. Every one of those is invisible in a diff and none of them
made any of our own tools complain.

That was found with `mlcroissant`, the reference validator. **`mlcroissant` was
not declared in `requirements.txt`, `requirements.lock` or
`requirements-optional.txt`, and is not installed on the build machine.** So the
check that stands between us and publishing a broken record was a thing somebody
once ran by hand, on a machine where it no longer exists. That is the same shape
as every control this project has already been bitten by: a check that cannot
fire is worse than no check, because it converts an unmonitored risk into a
monitored one nobody re-examines.

So this file does two things:

1. **A structural check that needs nothing but the standard library**, so it
   always runs: in CI, on the build machine, on a contributor's laptop.
2. **Defers to `mlcroissant` when it is present**, because the reference
   implementation is the authority and our reading of the specification is not.

Neither replaces the other. The structural check catches the errors we have
actually made; `mlcroissant` catches the ones we have not thought of. A run
reports plainly which of the two it managed, and `--require-reference` turns a
missing validator into a failure rather than a note, which is what a release
gate wants.

Usage::

    python validate_croissant.py path/to/croissant.json
    python validate_croissant.py path/to/croissant.json --require-reference
"""
from __future__ import annotations

import argparse
import json
import pathlib
import sys

#: The specification version our records claim. Recorded here so a bump is a
#: deliberate edit rather than a drift.
CONFORMS_TO = "http://mlcommons.org/croissant/1.0"

#: Terms the record cannot describe itself without. Not the whole map: these
#: are the ones whose absence makes a consumer misread the document rather than
#: merely lose detail.
REQUIRED_CONTEXT = (
    "@language", "@vocab", "cr", "sc", "dct", "citeAs", "column",
    "conformsTo", "data", "dataType", "field", "fileObject", "fileSet",
    "fileProperty", "includes", "isLiveDataset", "key", "md5", "parentField",
    "recordSet", "references", "regex", "repeated", "replace", "separator",
    "source", "subField", "transform",
)

#: Properties every published dataset record needs. `license` and `citeAs` are
#: here because this corpus exists to argue that licensing should be traceable,
#: and a record of ours that omits its own licence would be the wrong artefact
#: to ship under that argument.
REQUIRED_TOP = ("@context", "@type", "conformsTo", "name", "description",
                "license", "url", "version", "datePublished", "distribution",
                "recordSet", "citeAs")


def problems_with_context(ctx: object) -> list[str]:
    if not isinstance(ctx, dict):
        return ["@context is not an object, so no term resolves"]
    missing = [t for t in REQUIRED_CONTEXT if t not in ctx]
    if missing:
        return [f"@context is missing {len(missing)} term(s): "
                f"{', '.join(missing[:8])}"
                + (" ..." if len(missing) > 8 else "")]
    return []


def problems_with_distribution(dist: object) -> list[str]:
    """Every FileSet must say what it is contained in, and no glob may be bare.

    `containedIn` is what stops a FileSet meaning "every file anywhere". The
    cover FileSet that swallowed 769 arm shards did so because its glob was
    evaluated against the whole release rather than against one archive.
    """
    if not isinstance(dist, list) or not dist:
        return ["distribution is empty, so the record describes no files"]

    out = []
    ids = set()
    file_objects = set()
    for i, entry in enumerate(dist):
        if not isinstance(entry, dict):
            out.append(f"distribution[{i}] is not an object")
            continue
        eid = entry.get("@id")
        kind = entry.get("@type")
        if not eid:
            out.append(f"distribution[{i}] has no @id, so nothing can "
                       f"reference it")
            continue
        if eid in ids:
            out.append(f"distribution: @id {eid!r} appears more than once")
        ids.add(eid)
        if kind == "cr:FileObject":
            file_objects.add(eid)
            if not entry.get("contentUrl") and not entry.get("containedIn"):
                out.append(f"{eid}: a FileObject with neither contentUrl nor "
                           f"containedIn cannot be fetched")
        elif kind == "cr:FileSet":
            if "containedIn" not in entry:
                out.append(f"{eid}: a FileSet with no containedIn matches "
                           f"across the whole release rather than inside one "
                           f"archive")
            if not entry.get("includes"):
                out.append(f"{eid}: a FileSet with no includes glob selects "
                           f"nothing")
            if not entry.get("encodingFormat"):
                out.append(f"{eid}: no encodingFormat, so a consumer must "
                           f"guess how to read it")
        else:
            out.append(f"{eid}: unexpected @type {kind!r}")

    # A containedIn pointing at nothing is a dangling reference, and the record
    # still parses as JSON, so only a check like this one finds it.
    for entry in dist:
        if not isinstance(entry, dict):
            continue
        for ref in as_list(entry.get("containedIn")):
            target = ref.get("@id") if isinstance(ref, dict) else ref
            if target and target not in ids:
                out.append(f"{entry.get('@id')}: containedIn names "
                           f"{target!r}, which is not in distribution")
    return out


def as_list(value: object) -> list:
    if value is None:
        return []
    return value if isinstance(value, list) else [value]


def problems_with_record_sets(record_sets: object, dist_ids: set[str]) -> list[str]:
    """A record set needs fields, and every field needs a resolvable source."""
    if not isinstance(record_sets, list) or not record_sets:
        return ["recordSet is empty. A record with no record set describes "
                "files but no data, which is what made an earlier version of "
                "this record load as nothing"]

    out = []
    for rs in record_sets:
        if not isinstance(rs, dict):
            out.append("a recordSet entry is not an object")
            continue
        rid = rs.get("@id", "<no @id>")
        fields = rs.get("field")
        if not isinstance(fields, list) or not fields:
            out.append(f"{rid}: no fields")
            continue
        for field in fields:
            if not isinstance(field, dict):
                out.append(f"{rid}: a field is not an object")
                continue
            fid = field.get("@id", "<no @id>")
            if not field.get("dataType"):
                out.append(f"{fid}: no dataType")
            source = field.get("source")
            if not source:
                out.append(f"{fid}: no source, so nothing populates it")
                continue
            named = (source.get("fileSet") or source.get("fileObject")
                     or source.get("field")) if isinstance(source, dict) else None
            target = named.get("@id") if isinstance(named, dict) else named
            if target and "/" not in str(target) and target not in dist_ids:
                out.append(f"{fid}: source names {target!r}, which is not in "
                           f"distribution")
    return out


def structural_problems(doc: dict) -> list[str]:
    out = []
    for key in REQUIRED_TOP:
        if key not in doc:
            out.append(f"missing top-level {key!r}")
    if doc.get("@type") not in ("sc:Dataset", "Dataset"):
        out.append(f"@type is {doc.get('@type')!r}, expected 'sc:Dataset'")
    if doc.get("conformsTo") != CONFORMS_TO:
        out.append(f"conformsTo is {doc.get('conformsTo')!r}, expected "
                   f"{CONFORMS_TO!r}")
    out += problems_with_context(doc.get("@context"))
    out += problems_with_distribution(doc.get("distribution"))
    dist_ids = {e.get("@id") for e in as_list(doc.get("distribution"))
                if isinstance(e, dict)}
    out += problems_with_record_sets(doc.get("recordSet"), dist_ids)
    return out


def reference_problems(path: pathlib.Path) -> tuple[bool, list[str]]:
    """Run `mlcroissant` if it is installed. Returns (ran, problems)."""
    try:
        import mlcroissant as mlc
    except ImportError:
        return False, []
    try:
        mlc.Dataset(jsonld=str(path))
    except Exception as e:  # noqa: BLE001 - any failure is a failure to report
        return True, [f"{type(e).__name__}: {e}"]
    return True, []


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("record", help="a croissant.json")
    ap.add_argument("--require-reference", action="store_true",
                    help="fail when mlcroissant is not installed, rather than "
                         "reporting that only the structural check ran")
    args = ap.parse_args(argv)

    sys.stdout.reconfigure(line_buffering=True)
    path = pathlib.Path(args.record)
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as e:
        print(f"cannot read {path}: {e}", file=sys.stderr)
        return 1

    structural = structural_problems(doc)
    if structural:
        print(f"structural check: {len(structural)} problem(s)")
        for p in structural:
            print(f"  {p}")
    else:
        print("structural check: passed")

    ran, reference = reference_problems(path)
    if ran:
        if reference:
            print(f"mlcroissant: {len(reference)} problem(s)")
            for p in reference:
                print(f"  {p}")
        else:
            print("mlcroissant: passed")
    else:
        print("mlcroissant: NOT INSTALLED, so the authoritative check did not "
              "run. `pip install -r requirements-optional.txt`")
        if args.require_reference:
            print("\nrefusing: a release gate needs the reference validator, "
                  "not our reading of the specification", file=sys.stderr)
            return 1

    if structural or reference:
        return 1
    print(f"\n{path} is publishable"
          + ("" if ran else ", as far as the structural check can tell"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

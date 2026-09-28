#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Migrate result-v1 documents off `provenance.plugins[].route`.

`route` carried two facts at once: what pins the bytes that ran, and what those
bytes could reach. The two agree for a plain container and for a locally
installed program, and they disagree for an entry that names an image and runs
a host adapter against a service, where the image digest names the subject and
what executed was a script on the host with the full network. `route` is gone
and `pinned_by` plus `isolation` replace it.

This script is the migration, kept rather than run once and thrown away: it is
how the next person reproduces what happened to these files, and how a document
that arrives on the old shape is brought forward.

It is idempotent. A document already carrying both new fields is left alone and
counted as such. A document carrying neither `route` nor the new pair is
refused rather than guessed at, because inventing `sandbox-no-network` for a
run nobody described is exactly the flattering default this change exists to
remove.

    python3 tools/migrations/result_v1_route_split.py results/v1
    python3 tools/migrations/result_v1_route_split.py --check results/v1
"""

import argparse
import json
import os
import pathlib
import sys

# A document is small and there are two dozen of them, but a corrupt or hostile
# file must not be able to exhaust this process before it can say so.
MAX_BYTES = 8 * 1024 * 1024


class Refused(Exception):
    """A document this script will not guess at."""


def migrated(plugin: dict, network_reachable: bool) -> dict:
    """One plugin entry, brought onto the two-field shape."""
    has_new = "pinned_by" in plugin and "isolation" in plugin
    if has_new:
        if "route" in plugin:
            raise Refused(
                "carries both the old `route` and the new pair, so it is not "
                "clear which one a reader is meant to believe"
            )
        return plugin
    if "pinned_by" in plugin or "isolation" in plugin:
        raise Refused(
            "carries one half of the new pair and not the other; both are "
            "required and neither has a defensible default"
        )
    route = plugin.get("route")
    if route == "local":
        # The digest beside it is the hash of a file on one machine, and a
        # program installed here ran with this machine's network.
        pinned_by, isolation = "executable-hash", "host"
    elif route == "container":
        # A container is started with no network, EXCEPT where the document
        # itself says the network was reachable. That combination is the
        # service case: the image named the subject while a host adapter posted
        # to an instance, and calling it a sandbox would be the one migration
        # that leaves a reader worse off than the field it replaced.
        pinned_by = "image-digest"
        isolation = "remote-service" if network_reachable else "sandbox-no-network"
    else:
        raise Refused(f"names route {route!r}, which is neither container nor local")
    out = {k: v for k, v in plugin.items() if k != "route"}
    out["pinned_by"] = pinned_by
    out["isolation"] = isolation
    return out


def convert(doc: dict) -> tuple[dict, bool]:
    """The whole document, and whether anything actually changed."""
    provenance = doc.get("provenance")
    if not isinstance(provenance, dict):
        raise Refused("has no `provenance` object, so it is not a result-v1 document")
    plugins = provenance.get("plugins")
    if not isinstance(plugins, list):
        raise Refused("has no `provenance.plugins` list")
    reachable = bool(provenance.get("network_reachable", False))
    rebuilt = []
    for i, plugin in enumerate(plugins):
        if not isinstance(plugin, dict):
            raise Refused(f"plugin {i} is not an object")
        try:
            rebuilt.append(migrated(plugin, reachable))
        except Refused as e:
            raise Refused(f"plugin {i} ({plugin.get('name', 'unnamed')!r}) {e}") from e
    changed = rebuilt != plugins
    if changed:
        provenance["plugins"] = rebuilt
    return doc, changed


def rewrite(path: pathlib.Path, check_only: bool) -> bool:
    size = path.stat().st_size
    if size > MAX_BYTES:
        raise Refused(f"is {size} bytes, past the {MAX_BYTES} byte ceiling for a result")
    text = path.read_text(encoding="utf-8")
    try:
        doc = json.loads(text)
    except json.JSONDecodeError as e:
        raise Refused(f"is not readable JSON: {e}") from e
    if not isinstance(doc, dict):
        raise Refused("is not a JSON object")
    doc, changed = convert(doc)
    if not changed or check_only:
        return changed
    # The shipped documents are written by `generators/emit_results.py` with
    # sorted keys, two-space indent and a trailing newline. Matching that keeps
    # the diff to the fields that actually moved.
    new = json.dumps(doc, indent=2, sort_keys=True) + "\n"
    tmp = path.with_name(path.name + ".part")
    try:
        tmp.write_text(new, encoding="utf-8")
        os.replace(tmp, path)
    finally:
        tmp.unlink(missing_ok=True)
    return True


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("paths", nargs="+", help="result-v1 documents, or directories of them")
    ap.add_argument(
        "--check",
        action="store_true",
        help="report what would change and write nothing; exit 1 if anything would",
    )
    args = ap.parse_args(argv)

    files: list[pathlib.Path] = []
    for raw in args.paths:
        p = pathlib.Path(raw)
        if p.is_dir():
            files.extend(sorted(q for q in p.rglob("*.json") if q.is_file()))
        elif p.is_file():
            files.append(p)
        else:
            print(f"{p}: no such file or directory", file=sys.stderr)
            return 2
    if not files:
        # A walk that found nothing passes every check by never reaching one.
        print("no .json documents found in the paths given", file=sys.stderr)
        return 2

    changed, refused = 0, 0
    for f in files:
        try:
            if rewrite(f, args.check):
                changed += 1
                print(f"{'would migrate' if args.check else 'migrated'}: {f}")
        except Refused as e:
            refused += 1
            print(f"REFUSED {f}: {e}", file=sys.stderr)
        except OSError as e:
            refused += 1
            print(f"REFUSED {f}: could not be read or written: {e}", file=sys.stderr)

    print(f"{len(files)} document(s), {changed} changed, {refused} refused")
    if refused:
        return 1
    return 1 if (args.check and changed) else 0


if __name__ == "__main__":
    raise SystemExit(main())

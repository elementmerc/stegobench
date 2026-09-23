#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Score one image with StegaShield, and print the likelihood.

WHY THIS TOOL IS SHAPED DIFFERENTLY FROM THE REST
--------------------------------------------------
Every other detector here is a command: hand it a file, read what it prints.
StegaShield is a service. It runs as a container serving HTTP, and scoring an
image means posting it to a running instance.

That changes who is responsible for what. The licence token is consumed when
the CONTAINER STARTS, not on each request, so this adapter never sees it and
never should. Whoever runs the service supplies the token to the container;
this only asks the service a question.

THE ENDPOINT IS SUPPLIED, NEVER DEFAULTED
-----------------------------------------
There is deliberately no default address. A benchmark that ships one scores
against whatever happens to answer on it, and "whatever answers on localhost"
is not a subject anybody can name in a result.

The image is NOT vendored anywhere in this repository. It is a third party's
artefact, referenced by digest and pulled by whoever runs it.

Usage:
    STEGASHIELD_ENDPOINT=http://host:3000/api/analyse stegashield_one.py <image>
"""
from __future__ import annotations

import json
import os
import pathlib
import sys
import urllib.error
import urllib.request

BOUNDARY = "----stegobench-boundary-7f3a"


def post_image(endpoint: str, path: pathlib.Path, timeout: int = 120) -> dict:
    """One multipart upload, built by hand to avoid a dependency."""
    body = b"".join([
        f"--{BOUNDARY}\r\n".encode(),
        f'Content-Disposition: form-data; name="image"; filename="{path.name}"\r\n'.encode(),
        b"Content-Type: application/octet-stream\r\n\r\n",
        path.read_bytes(),
        f"\r\n--{BOUNDARY}--\r\n".encode(),
    ])
    req = urllib.request.Request(
        endpoint,
        data=body,
        headers={"Content-Type": f"multipart/form-data; boundary={BOUNDARY}"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: stegashield_one.py <image>", file=sys.stderr)
        return 2

    endpoint = os.environ.get("STEGASHIELD_ENDPOINT", "").strip()
    if not endpoint:
        print(
            "STEGASHIELD_ENDPOINT is not set. This tool is a service, so the "
            "address of a running instance has to be supplied; there is no "
            "default, because a benchmark that ships one scores against "
            "whatever answers on it.",
            file=sys.stderr,
        )
        return 3

    path = pathlib.Path(argv[1])
    if not path.is_file():
        print(f"{path} is not a file", file=sys.stderr)
        return 1

    try:
        response = post_image(endpoint, path)
    except urllib.error.URLError as e:
        print(f"could not reach {endpoint}: {e}", file=sys.stderr)
        return 4
    except Exception as e:  # noqa: BLE001 - one image, and it says which
        print(f"{type(e).__name__}: {e}", file=sys.stderr)
        return 1

    value = response.get("stego_probability")
    if value is None:
        # Never fall back to zero. A missing field would read as the most
        # confident possible "clean", which is the opposite of not knowing.
        print(
            f"no stego_probability in the response: {json.dumps(response)[:200]}",
            file=sys.stderr,
        )
        return 1

    print(f"{float(value):.10f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))

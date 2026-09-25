#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for `tools/release/sign-artefacts.sh`, run against a stub cosign.

    python3 -m unittest discover -s tools/release -p 'test_*.py'

The script really runs here. What it calls is a shell stub on PATH that
records its arguments and can be told to fail on a chosen file, so the case
worth testing most is the one a real run would only reveal at tag time:
cosign dying on the fourth of nine artefacts. The script must exit non-zero
there, loudly, rather than handing a partly signed directory to the step that
publishes it.

The stub is not a cryptographic check and is not pretending to be one. What is
being tested is the script's control flow: that it signs every artefact,
SHA256SUMS among them, that it does not sign its own output, that it verifies
what it signed, and that every failure path refuses rather than continues.

Skipped where bash is not the shell that will run it: the release job runs on
Linux, while this suite also runs on the macOS and Windows runners.
"""
from __future__ import annotations

import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parent / "sign-artefacts.sh"

# A stub that writes the two files cosign would write, and refuses whichever
# file it is told to refuse. `verify-blob` answers yes unless told otherwise.
STUB = """#!/usr/bin/env bash
set -u
verb="$1"; shift
target="${!#}"
name="$(basename "${target}")"
case "${verb}" in
  sign-blob)
    if [ "${STUB_FAIL_SIGN:-}" = "${name}" ]; then
      echo "stub cosign: refusing to sign ${name}" >&2
      exit 1
    fi
    sig=""; cert=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --output-signature) sig="$2"; shift 2 ;;
        --output-certificate) cert="$2"; shift 2 ;;
        *) shift ;;
      esac
    done
    if [ "${STUB_EMPTY_SIG:-}" = "${name}" ]; then
      : > "${sig}"
    else
      echo "signature-of-${name}" > "${sig}"
    fi
    echo "certificate-of-${name}" > "${cert}"
    echo "${name}" >> "${STUB_LOG}.signed"
    ;;
  verify-blob)
    if [ "${STUB_FAIL_VERIFY:-}" = "${name}" ]; then
      echo "stub cosign: ${name} does not verify" >&2
      exit 1
    fi
    echo "${name}" >> "${STUB_LOG}.verified"
    ;;
  *)
    echo "stub cosign: unexpected verb ${verb}" >&2
    exit 2
    ;;
esac
exit 0
"""


def bash_is_available() -> bool:
    return shutil.which("bash") is not None and sys.platform != "win32"


@unittest.skipUnless(bash_is_available(), "sign-artefacts.sh needs a POSIX bash")
class SigningAStagedRelease(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        stub = self.bin / "cosign"
        stub.write_text(STUB, encoding="utf-8")
        stub.chmod(0o755)
        self.log = self.tmp / "calls"
        self.dist = self.tmp / "dist"
        self.dist.mkdir()

    def stage(self, *names: str, checksums: bool = True) -> None:
        for name in names:
            (self.dist / name).write_bytes(b"pretend this is a tarball")
        if checksums:
            (self.dist / "SHA256SUMS").write_text(
                "\n".join(f"0000  {name}" for name in names) + "\n",
                encoding="utf-8",
            )

    def run_script(self, *, cosign: bool = True, **stub_env: str):
        env = dict(os.environ)
        path = str(self.bin) if cosign else str(self.tmp / "empty")
        (self.tmp / "empty").mkdir(exist_ok=True)
        # A PATH holding only the stub directory, so nothing here can reach a
        # cosign that happens to be installed on the machine running the test.
        env["PATH"] = path + os.pathsep + "/usr/bin" + os.pathsep + "/bin"
        env["STUB_LOG"] = str(self.log)
        env.update(stub_env)
        return subprocess.run(
            ["bash", str(SCRIPT), str(self.dist)],
            capture_output=True, text=True, env=env, timeout=120,
        )

    def signed(self) -> list[str]:
        path = pathlib.Path(f"{self.log}.signed")
        return path.read_text(encoding="utf-8").split() if path.exists() else []

    def verified(self) -> list[str]:
        path = pathlib.Path(f"{self.log}.verified")
        return path.read_text(encoding="utf-8").split() if path.exists() else []

    # ── The path a real release takes ───────────────────────────────────────

    def test_every_artefact_is_signed_and_sha256sums_among_them(self) -> None:
        self.stage("a.tar.gz", "b.zip", "core.cdx.json")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            sorted(self.signed()),
            ["SHA256SUMS", "a.tar.gz", "b.zip", "core.cdx.json"],
        )
        self.assertEqual(sorted(self.verified()), sorted(self.signed()))
        for name in ("a.tar.gz", "b.zip", "core.cdx.json", "SHA256SUMS"):
            self.assertTrue((self.dist / f"{name}.sig").is_file())
            self.assertTrue((self.dist / f"{name}.pem").is_file())

    def test_it_does_not_sign_its_own_signatures(self) -> None:
        """Otherwise a second run would sign the first run's output."""
        self.stage("a.tar.gz")
        self.assertEqual(self.run_script().returncode, 0)
        first = sorted(self.signed())
        second = self.run_script()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(sorted(set(self.signed())), first)

    # ── Every way it is supposed to refuse ──────────────────────────────────

    def test_it_refuses_when_cosign_is_missing(self) -> None:
        self.stage("a.tar.gz")
        result = self.run_script(cosign=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cosign is not on PATH", result.stderr)

    def test_it_refuses_without_sha256sums(self) -> None:
        self.stage("a.tar.gz", checksums=False)
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("SHA256SUMS", result.stderr)
        self.assertEqual(self.signed(), [])

    def test_it_refuses_an_empty_directory(self) -> None:
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("SHA256SUMS", result.stderr)

    def test_it_refuses_a_missing_directory(self) -> None:
        shutil.rmtree(self.dist)
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no such directory", result.stderr)

    def test_a_failure_halfway_stops_the_release(self) -> None:
        """The case a real run would only reveal at tag time."""
        self.stage("a.tar.gz", "b.zip", "c.tar.gz")
        result = self.run_script(STUB_FAIL_SIGN="b.zip")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("could not sign b.zip", result.stderr)
        self.assertIn("Nothing has been published", result.stderr)
        self.assertNotIn("c.tar.gz", self.signed())

    def test_an_empty_signature_is_not_a_signature(self) -> None:
        self.stage("a.tar.gz")
        result = self.run_script(STUB_EMPTY_SIG="a.tar.gz")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("empty signature", result.stderr)

    def test_a_signature_that_does_not_verify_stops_the_release(self) -> None:
        self.stage("a.tar.gz", "b.zip")
        result = self.run_script(STUB_FAIL_VERIFY="b.zip")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not verify", result.stderr)

    def test_it_checks_the_identity_the_documented_command_checks(self) -> None:
        """The verification arguments are what a downloader is told to paste."""
        self.stage("a.tar.gz")
        self.assertEqual(self.run_script().returncode, 0)
        body = SCRIPT.read_text(encoding="utf-8")
        self.assertIn("elementmerc/stegobench", body)
        self.assertIn("token.actions.githubusercontent.com", body)

    def test_it_takes_exactly_one_argument(self) -> None:
        env = dict(os.environ)
        env["PATH"] = str(self.bin) + os.pathsep + env.get("PATH", "")
        env["STUB_LOG"] = str(self.log)
        result = subprocess.run(
            ["bash", str(SCRIPT)], capture_output=True, text=True, env=env,
            timeout=60,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly one argument", result.stderr)


if __name__ == "__main__":
    unittest.main()

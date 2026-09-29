#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the release workflow's signing path.

    python3 -m unittest discover -s tools/release -p 'test_*.py'

Two things about a release workflow cannot be checked by running it: the run
happens on a tag push, and a tag is a one-way door. So the properties worth
holding are held here, over the file's own text, on every CI run.

WHAT THIS FILE REFUSES TO LET ANYBODY DO QUIETLY

1. Reference an action by a mutable tag. `@v4` is whatever the owner of that
   repository last pointed the tag at, which is how the tj-actions compromise
   reached everybody who wrote it that way. Baseline Section 5.
2. Publish before signing. If the signing steps drift below
   `gh release create`, a half-failed run leaves a draft carrying artefacts
   nobody signed, which is the failure this whole change exists to prevent.
3. Leave SHA256SUMS unsigned. It is the file an attacker would most want to
   swap, because a checksum list anybody can regenerate proves only that the
   bytes match a list the same attacker wrote.
4. Fill a placeholder pin with something that looks like a hash. A wrong
   commit SHA that parses is far worse than an obvious blank, so the
   placeholder is deliberately not hexadecimal and is guarded by a pre-flight
   step that refuses the release while it is still there.

The checks read the workflow as text rather than as parsed YAML, because the
trailing version comment on a pin and the ORDER of steps are both properties
of the text, and because PyYAML is not in `requirements.txt` and this suite
runs in CI from that file alone. The one test that genuinely needs a parser
skips when it is absent, and says so.
"""
from __future__ import annotations

import os
import pathlib
import re
import stat
import unittest

REPO = pathlib.Path(__file__).resolve().parents[2]
WORKFLOWS = REPO / ".github" / "workflows"
RELEASE_YML = WORKFLOWS / "release.yml"
RELEASING_MD = REPO / "tools" / "release" / "RELEASING.md"
SIGN_SCRIPT = REPO / "tools" / "release" / "sign-artefacts.sh"

# The one ref allowed to stand in for a commit SHA. Uppercase and full of
# non-hexadecimal letters, so it can never be mistaken for a hash by a reader
# or by a regular expression.
PLACEHOLDER = "PIN-ME-SEE-RELEASING-MD"

USES = re.compile(r"^\s*(?:-\s+)?uses:\s*(?P<ref>\S+)(?P<rest>.*)$", re.MULTILINE)
FORTY_HEX = re.compile(r"^[0-9a-f]{40}$")
VERSION_COMMENT = re.compile(r"#\s*v\d+\.\d+\.\d+")


def job_block(text: str, name: str) -> str:
    """The lines of one job, from its key to the next key at the same indent."""
    header = f"  {name}:\n"
    start = text.index("\n" + header) + 1
    body_at = start + len(header)
    following = re.search(r"^  [a-z][a-z0-9-]*:\s*$", text[body_at:], re.MULTILINE)
    end = body_at + (following.start() if following else len(text) - body_at)
    return text[start:end]


class ActionsArePinnedByCommit(unittest.TestCase):
    """Every `uses:` in every workflow, not only the release one."""

    def test_every_action_is_a_commit_sha_or_a_loud_placeholder(self) -> None:
        for workflow in sorted(WORKFLOWS.glob("*.yml")):
            text = workflow.read_text(encoding="utf-8")
            for match in USES.finditer(text):
                ref = match.group("ref")
                with self.subTest(workflow=workflow.name, uses=ref):
                    self.assertIn("@", ref, "an action reference must carry a ref")
                    pin = ref.split("@", 1)[1]
                    if pin == PLACEHOLDER:
                        continue
                    self.assertRegex(
                        pin, FORTY_HEX,
                        "pinned by something other than a full commit SHA. A tag "
                        "can be repointed at any commit by whoever owns it.",
                    )

    def test_one_action_is_not_pinned_to_two_different_commits(self) -> None:
        """Two pins for one action mean somebody wrote a hash from memory.

        A hash cannot be checked by reading it, which is exactly why writing
        one from memory is tempting and exactly why it is dangerous: a
        plausible wrong hash looks more checked than a blank does. It happened
        while this file was being written, in a new workflow that pinned
        `actions/upload-artifact` to a hash nothing else in the repository
        agreed with.

        Disagreement is not proof of invention, since a repository can
        legitimately carry two versions of one action. So this reports the
        disagreement and names both, rather than deciding which is wrong.
        """
        pins: dict[str, set[str]] = {}
        for workflow in sorted(WORKFLOWS.glob("*.yml")):
            for match in USES.finditer(workflow.read_text(encoding="utf-8")):
                ref = match.group("ref")
                if "@" not in ref:
                    continue
                action, pin = ref.split("@", 1)
                if not FORTY_HEX.match(pin):
                    continue
                pins.setdefault(action, set()).add(pin)
        for action, seen in sorted(pins.items()):
            with self.subTest(action=action):
                self.assertEqual(
                    len(seen), 1,
                    f"{action} is pinned to {len(seen)} different commits: "
                    f"{sorted(seen)}. One of them may have been written from "
                    f"memory. Resolve both against the upstream repository "
                    f"before trusting either.",
                )

    def test_every_real_pin_records_the_version_it_is(self) -> None:
        """A bare hash is unreviewable. The trailing comment says what it is."""
        for workflow in sorted(WORKFLOWS.glob("*.yml")):
            text = workflow.read_text(encoding="utf-8")
            for match in USES.finditer(text):
                pin = match.group("ref").split("@", 1)[-1]
                if not FORTY_HEX.match(pin):
                    continue
                with self.subTest(workflow=workflow.name, uses=match.group("ref")):
                    self.assertRegex(
                        match.group("rest"), VERSION_COMMENT,
                        "a commit pin needs the version in a trailing comment",
                    )

    def test_a_placeholder_can_never_be_mistaken_for_a_hash(self) -> None:
        self.assertFalse(FORTY_HEX.match(PLACEHOLDER.lower()))
        self.assertNotEqual(len(PLACEHOLDER), 40)

    def test_a_placeholder_is_refused_by_the_pre_flight(self) -> None:
        """A release carrying one must fail in the first minute, explained."""
        text = RELEASE_YML.read_text(encoding="utf-8")
        placeholders = text.count(f"@{PLACEHOLDER}")
        if not placeholders:
            self.skipTest("no placeholder pins remain, so nothing to guard")
        preflight = job_block(text, "preflight")
        self.assertIn("grep", preflight)
        # Written with a bracket so the guard's own line does not match itself.
        self.assertIn("PIN-ME-SEE-[R]ELEASING-MD", preflight)
        self.assertIn("exit 1", preflight)

    def test_a_placeholder_is_documented_where_a_human_will_look(self) -> None:
        text = RELEASE_YML.read_text(encoding="utf-8")
        if f"@{PLACEHOLDER}" not in text:
            self.skipTest("no placeholder pins remain, so nothing to document")
        self.assertIn(PLACEHOLDER, RELEASING_MD.read_text(encoding="utf-8"))
        self.assertIn("Filling in the signing pins",
                      RELEASING_MD.read_text(encoding="utf-8"))


def without_comments(block: str) -> str:
    """The block with whole-line comments dropped.

    The ordering checks below compare where things appear, and this file
    explains itself at length. A comment describing the publish step is not a
    publish step, and counting it as one would make the order look wrong while
    the workflow was right.
    """
    return "\n".join(
        line for line in block.splitlines() if not line.lstrip().startswith("#")
    )


class NothingIsPublishedBeforeItIsSigned(unittest.TestCase):
    def setUp(self) -> None:
        self.text = RELEASE_YML.read_text(encoding="utf-8")
        self.raw = job_block(self.text, "release")
        self.release = without_comments(self.raw)

    def test_exactly_one_job_publishes(self) -> None:
        """A second publish path is a second path that could skip signing."""
        self.assertEqual(without_comments(self.text).count("gh release create"), 1)
        self.assertIn("gh release create", self.release)

    def test_the_draft_is_created_after_every_artefact_is_signed(self) -> None:
        self.assertIn(
            "tools/release/sign-artefacts.sh", self.release,
            "the release job no longer signs anything",
        )
        sign = self.release.index("tools/release/sign-artefacts.sh")
        publish = self.release.index("gh release create")
        self.assertLess(
            sign, publish,
            "signing moved below the publish step. A run that fails halfway "
            "would then leave a draft carrying unsigned artefacts.",
        )

    def test_the_provenance_attestation_covers_the_published_files(self) -> None:
        self.assertIn(
            "actions/attest-build-provenance@", self.release,
            "the release job no longer attests how the artefacts were built",
        )
        attest = self.release.index("actions/attest-build-provenance@")
        checksums = self.release.index("SHA256SUMS")
        publish = self.release.index("gh release create")
        self.assertLess(checksums, attest, "SHA256SUMS must exist to be attested")
        self.assertLess(attest, publish)
        self.assertIn("subject-path: dist/*", self.release)

    def test_the_release_job_can_sign_and_attest(self) -> None:
        self.assertIn("id-token: write", self.release)
        self.assertIn("attestations: write", self.release)

    def test_what_was_uploaded_is_reconciled_against_what_was_signed(self) -> None:
        reconcile = self.release.index("--json assets")
        self.assertGreater(reconcile, self.release.index("gh release create"))
        self.assertIn("diff -u", self.release)


class TheSigningScriptIsUsable(unittest.TestCase):
    def test_it_exists(self) -> None:
        self.assertTrue(SIGN_SCRIPT.is_file())

    @unittest.skipIf(
        os.name == "nt",
        "Windows has no executable bit, and git does not fabricate one on "
        "checkout. The bit is what the release runner needs, and that is "
        "Linux, so it is asserted where it means something.",
    )
    def test_it_is_executable(self) -> None:
        mode = SIGN_SCRIPT.stat().st_mode
        self.assertTrue(mode & stat.S_IXUSR, "sign-artefacts.sh is not executable")

    def test_it_refuses_to_run_without_sha256sums(self) -> None:
        """The checksum file is signed explicitly, not incidentally."""
        body = SIGN_SCRIPT.read_text(encoding="utf-8")
        self.assertIn('[ -s "${dist}/SHA256SUMS" ]', body)

    def test_it_verifies_what_it_signed(self) -> None:
        body = SIGN_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("cosign sign-blob", body)
        self.assertIn("cosign verify-blob", body)


class ADownloaderIsToldHowToCheck(unittest.TestCase):
    """Instructions a stranger can paste, in the two places one would look."""

    def test_the_readme_carries_both_verification_commands(self) -> None:
        readme = (REPO / "README.md").read_text(encoding="utf-8")
        self.assertIn("cosign verify-blob", readme)
        self.assertIn("gh attestation verify", readme)
        self.assertIn("--certificate-identity-regexp", readme)
        self.assertIn("--certificate-oidc-issuer", readme)

    def test_security_md_says_what_a_signature_does_and_does_not_prove(self) -> None:
        security = (REPO / "SECURITY.md").read_text(encoding="utf-8")
        self.assertIn("cosign verify-blob", security)
        self.assertIn("gh attestation verify", security)

    def test_the_identity_the_docs_check_is_the_one_the_workflow_signs_with(self) -> None:
        """A verification command naming a different workflow always fails."""
        readme = (REPO / "README.md").read_text(encoding="utf-8")
        # The backslashes are regular expression escaping in the documented
        # command; dropping them is what makes the path readable here.
        block = readme[readme.index("--certificate-identity-regexp"):][:400]
        block = block.replace("\\", "")
        self.assertIn("elementmerc/stegobench", block)
        self.assertIn("release.yml", block)
        # The same identity the signing script verifies against, so a reader
        # pasting the documented command checks what the workflow asserted.
        script = SIGN_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("github.com/elementmerc/stegobench", script.replace("\\", ""))
        self.assertIn("token.actions.githubusercontent.com", readme)


class TheWorkflowIsValidYaml(unittest.TestCase):
    def test_it_parses(self) -> None:
        # Imported without a guard since 2026-09-29, when PyYAML entered
        # `requirements.txt`. It used to skip when the parser was absent, and
        # the skip was honest then. Now an absent parser means the pinned set
        # did not install, and reporting that as "nothing to check here" would
        # turn a broken environment into a green run.
        import yaml  # noqa: PLC0415

        for workflow in sorted(WORKFLOWS.glob("*.yml")):
            with self.subTest(workflow=workflow.name):
                with workflow.open(encoding="utf-8") as handle:
                    parsed = yaml.safe_load(handle)
                self.assertIn("jobs", parsed)


if __name__ == "__main__":
    unittest.main()

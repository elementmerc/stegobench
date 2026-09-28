#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the `route` to `pinned_by` plus `isolation` migration.

A migration script is run once against files nobody can regenerate, so its
refusals matter more than its successes: a wrong guess writes a plausible
falsehood into a published document and nothing downstream can tell.
"""

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from result_v1_route_split import Refused, convert, main, migrated  # noqa: E402


def doc(plugins, network_reachable=False):
    return {"provenance": {"plugins": plugins, "network_reachable": network_reachable}}


class Mapping(unittest.TestCase):
    def test_a_container_with_no_network_is_a_pinned_sandbox(self):
        out = migrated({"name": "x", "route": "container"}, network_reachable=False)
        self.assertEqual(out["pinned_by"], "image-digest")
        self.assertEqual(out["isolation"], "sandbox-no-network")
        self.assertNotIn("route", out)

    def test_a_container_beside_a_reachable_network_is_the_service_case(self):
        # The shape that made this change necessary: the image names the
        # subject and a host adapter did the running, so calling it a sandbox
        # would be the one migration that misleads worse than `route` did.
        out = migrated({"name": "x", "route": "container"}, network_reachable=True)
        self.assertEqual(out["pinned_by"], "image-digest")
        self.assertEqual(out["isolation"], "remote-service")

    def test_a_local_binary_is_hashed_and_unsandboxed(self):
        out = migrated({"name": "x", "route": "local"}, network_reachable=True)
        self.assertEqual(out["pinned_by"], "executable-hash")
        self.assertEqual(out["isolation"], "host")

    def test_other_fields_survive_untouched(self):
        out = migrated(
            {"name": "x", "image": "a@sha256:b", "determinism": "exact", "route": "local"},
            network_reachable=False,
        )
        self.assertEqual(out["image"], "a@sha256:b")
        self.assertEqual(out["determinism"], "exact")


class Refusals(unittest.TestCase):
    def test_an_unknown_route_is_refused_rather_than_guessed(self):
        with self.assertRaises(Refused):
            migrated({"name": "x", "route": "somewhere-else"}, False)

    def test_a_plugin_with_no_route_at_all_is_refused(self):
        with self.assertRaises(Refused):
            migrated({"name": "x"}, False)

    def test_half_the_new_pair_is_refused(self):
        with self.assertRaises(Refused):
            migrated({"name": "x", "pinned_by": "image-digest"}, False)

    def test_both_shapes_at_once_is_refused(self):
        with self.assertRaises(Refused):
            migrated(
                {"name": "x", "route": "local", "pinned_by": "a", "isolation": "b"},
                False,
            )

    def test_a_document_without_provenance_is_refused(self):
        with self.assertRaises(Refused):
            convert({"schema": "stegobench/result-v1"})

    def test_a_plugin_that_is_not_an_object_is_refused(self):
        with self.assertRaises(Refused):
            convert(doc(["not an object"]))

    def test_the_refusal_names_which_plugin_failed(self):
        with self.assertRaises(Refused) as caught:
            convert(doc([{"name": "ok", "route": "local"}, {"name": "bad"}]))
        self.assertIn("'bad'", str(caught.exception))


class WholeDocuments(unittest.TestCase):
    def test_an_already_migrated_document_reports_no_change(self):
        _, changed = convert(
            doc([{"name": "x", "pinned_by": "image-digest", "isolation": "host"}])
        )
        self.assertFalse(changed)

    def test_an_empty_plugin_list_is_not_an_error(self):
        _, changed = convert(doc([]))
        self.assertFalse(changed)


class CommandLine(unittest.TestCase):
    def write(self, tmp, name, plugins, reachable=False):
        p = pathlib.Path(tmp) / name
        p.write_text(json.dumps(doc(plugins, reachable)), encoding="utf-8")
        return p

    def test_a_directory_is_walked_and_rewritten_once(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = self.write(tmp, "a.json", [{"name": "x", "route": "container"}])
            self.assertEqual(main([tmp]), 0)
            written = json.loads(p.read_text(encoding="utf-8"))
            self.assertEqual(
                written["provenance"]["plugins"][0]["isolation"], "sandbox-no-network"
            )
            # Idempotent: a second pass changes nothing and leaves no `.part`.
            self.assertEqual(main([tmp]), 0)
            self.assertEqual(json.loads(p.read_text(encoding="utf-8")), written)
            self.assertEqual(list(pathlib.Path(tmp).glob("*.part")), [])

    def test_check_writes_nothing_and_exits_one_when_work_remains(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = self.write(tmp, "a.json", [{"name": "x", "route": "local"}])
            before = p.read_text(encoding="utf-8")
            self.assertEqual(main(["--check", tmp]), 1)
            self.assertEqual(p.read_text(encoding="utf-8"), before)

    def test_check_exits_zero_when_nothing_is_left_to_do(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.write(tmp, "a.json", [{"name": "x", "pinned_by": "a", "isolation": "b"}])
            self.assertEqual(main(["--check", tmp]), 0)

    def test_a_refused_document_exits_one_and_leaves_the_file_alone(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = self.write(tmp, "a.json", [{"name": "x", "route": "elsewhere"}])
            before = p.read_text(encoding="utf-8")
            self.assertEqual(main([tmp]), 1)
            self.assertEqual(p.read_text(encoding="utf-8"), before)

    def test_unreadable_json_is_refused_rather_than_crashing(self):
        with tempfile.TemporaryDirectory() as tmp:
            (pathlib.Path(tmp) / "a.json").write_text("{not json", encoding="utf-8")
            self.assertEqual(main([tmp]), 1)

    def test_a_path_that_does_not_exist_is_named(self):
        self.assertEqual(main([str(pathlib.Path(tempfile.gettempdir()) / "nope-xyzzy")]), 2)

    def test_a_directory_with_no_documents_is_refused_rather_than_called_clean(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(main([tmp]), 2)

    def test_an_oversized_document_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = pathlib.Path(tmp) / "a.json"
            p.write_text(" " * (9 * 1024 * 1024), encoding="utf-8")
            self.assertEqual(main([tmp]), 1)


if __name__ == "__main__":
    unittest.main()

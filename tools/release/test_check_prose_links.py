#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the published-prose link checker.

Nothing here touches the network. Every test injects a fetcher or patches the
module's, because a test that needed a live page would fail on an aeroplane and
pass when the thing it tests is broken. That is not hypothetical here: the
post-publish verifier's tests next door did exactly that until 2026-09-25,
because the injection point was bound as a default argument.
"""

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import check_prose_links as links  # noqa: E402
import verify_published as vp  # noqa: E402


def answering(status=200, body=""):
    return lambda url: vp.Fetched(status, body)


class TidyTests(unittest.TestCase):
    def test_a_full_stop_ending_the_sentence_is_not_part_of_the_address(self):
        self.assertEqual(links.tidy("https://e/a."), "https://e/a")

    def test_a_comma_in_a_list_is_not_part_of_the_address(self):
        self.assertEqual(links.tidy("https://e/a,"), "https://e/a")

    def test_an_unmatched_closing_bracket_is_dropped(self):
        self.assertEqual(links.tidy("https://e/a)"), "https://e/a")

    def test_a_matched_bracket_pair_is_kept(self):
        # Wikipedia-shaped addresses really do carry brackets.
        self.assertEqual(links.tidy("https://e/A_(b)"), "https://e/A_(b)")

    def test_an_address_that_is_only_punctuation_survives_to_empty(self):
        self.assertEqual(links.tidy("..."), "")


class SkipTests(unittest.TestCase):
    def test_an_ordinary_address_is_checked(self):
        self.assertIsNone(links.why_skipped("https://github.com/x/y"))

    def test_a_private_address_is_skipped_because_the_guides_print_them(self):
        # docs/guide/http-detector.md shows a reader the refusal they get for
        # one. Asking would fail on every single run.
        for url in ("http://172.24.0.2:3000/api/analyze",
                    "http://10.1.2.3:3000/api/score",
                    "http://127.0.0.1:8080/",
                    "http://192.168.1.1/"):
            self.assertIsNotNone(links.why_skipped(url), url)

    def test_a_reserved_example_domain_is_skipped(self):
        for url in ("https://example.com/a", "https://sub.example.org/a",
                    "http://localhost:3000/"):
            self.assertIsNotNone(links.why_skipped(url), url)

    def test_a_template_is_skipped_rather_than_requested(self):
        self.assertIsNotNone(links.why_skipped("https://<host>:3000/api"))

    def test_a_bare_word_host_is_treated_as_an_example(self):
        self.assertIsNotNone(links.why_skipped("https://e/api/analyze"))

    def test_a_public_address_is_not_skipped(self):
        self.assertIsNone(links.why_skipped("https://8.8.8.8/"))


class GatherTests(unittest.TestCase):
    def repo(self, tmp, files):
        root = pathlib.Path(tmp)
        for name, text in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        return root

    def test_addresses_are_found_and_attributed_to_their_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, {
                "README.md": "see https://host-a.co/one and https://host-b.co/two.",
                "docs/guide/x.md": "also https://host-a.co/one here",
            })
            found = links.addresses_in(root)
        self.assertEqual(sorted(found), ["https://host-a.co/one", "https://host-b.co/two"])
        self.assertEqual(found["https://host-a.co/one"], ["README.md", "docs/guide/x.md"])

    def test_a_registry_entry_is_read_because_its_licence_url_matters_most(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, {
                "plugins/registry/corpora/x.toml": 'url = "https://licence-host.co/by/4.0/"',
            })
            self.assertIn("https://licence-host.co/by/4.0/", links.addresses_in(root))

    def test_a_source_file_is_not_read_because_it_promises_nobody_anything(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, {"src/main.rs": "// https://nobody-host.co/follows-this"})
            self.assertEqual(links.addresses_in(root), {})


class SpreadTests(unittest.TestCase):
    def test_no_single_host_can_consume_the_whole_sample(self):
        # A flat truncation would spend it all on github.com and prove nothing
        # about the licence addresses, which are the ones that matter.
        urls = [f"https://github.com/{i}" for i in range(50)]
        urls += ["https://creativecommons.org/licenses/by/4.0/"]
        chosen = links.spread(urls, 4)
        self.assertIn("https://creativecommons.org/licenses/by/4.0/", chosen)
        self.assertEqual(len(chosen), 4)

    def test_it_is_deterministic(self):
        urls = [f"https://h{i % 3}.co/{i}" for i in range(20)]
        self.assertEqual(links.spread(urls, 7), links.spread(list(reversed(urls)), 7))

    def test_asking_for_more_than_exists_returns_everything_once(self):
        urls = ["https://host-a.co/1", "https://host-b.co/2"]
        self.assertEqual(sorted(links.spread(urls, 99)), urls)


class LookTests(unittest.TestCase):
    def test_a_page_that_answers_is_alive(self):
        self.assertEqual(links.look("https://host-e.co/", answering(200))["verdict"], "alive")

    def test_a_404_is_dead_which_is_the_whole_point(self):
        row = links.look("https://host-e.co/", answering(404))
        self.assertEqual(row["verdict"], "dead")
        self.assertEqual(row["status"], 404)

    def test_a_rate_limit_is_about_the_host_and_not_about_our_prose(self):
        for status in (429, 500, 502, 503):
            self.assertEqual(
                links.look("https://host-e.co/", answering(status))["verdict"],
                "unsettled", status)

    def test_a_host_that_gives_no_answer_is_distinguished_from_a_404(self):
        def refusing(url):
            raise RuntimeError("no route to host")

        row = links.look("https://host-e.co/", refusing)
        self.assertEqual(row["verdict"], "unreachable")
        self.assertIn("no route to host", row["detail"])

    def test_a_redirect_counts_as_alive(self):
        # urllib follows redirects, so a 3xx reaching here at all is unusual,
        # but a moved page is still a page a reader arrives at.
        self.assertEqual(links.look("https://host-e.co/", answering(301))["verdict"], "alive")


class MainTests(unittest.TestCase):
    def run_main(self, argv, fetcher):
        real = links.fetch
        links.fetch = fetcher
        try:
            return links.main(argv)
        finally:
            links.fetch = real

    def repo(self, tmp, body):
        root = pathlib.Path(tmp) / "repo"
        root.mkdir()
        (root / "README.md").write_text(body, encoding="utf-8")
        return root

    def test_a_dead_address_fails_the_run_and_the_record_names_the_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, "report bugs at https://host-a.co/issues")
            out = pathlib.Path(tmp) / "links.json"
            code = self.run_main(
                ["--root", str(root), "--out", str(out)], answering(404))
            self.assertEqual(code, 1)
            # Read inside the block: the record lives in the temporary
            # directory and goes away with it.
            written = json.loads(out.read_text())
        row = written["results"][0]
        self.assertEqual(row["verdict"], "dead")
        self.assertEqual(row["named_in"], ["README.md"])
        # Written although the run failed, because a record of a failed check
        # is more useful than no record and the exit code carries the verdict.
        self.assertEqual(written["distinct_addresses"], 1)

    def test_a_live_address_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, "see https://host-a.co/ok")
            self.assertEqual(
                self.run_main(["--root", str(root)], answering(200)), 0)

    def test_a_rate_limited_run_does_not_fail_but_does_not_claim_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, "see https://host-a.co/ok")
            # Nothing answered, so nothing was proven. Exiting zero here would
            # read as a clean bill of health for prose nobody checked.
            self.assertEqual(
                self.run_main(["--root", str(root)], answering(429)), 1)

    def test_prose_with_only_example_addresses_refuses_rather_than_passing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, "export E=http://172.24.0.2:3000/api/analyze")
            # Every address skipped means nothing was checked, and a run that
            # checked nothing must not exit zero.
            code = self.run_main(["--root", str(root)], answering(200))
            self.assertEqual(code, 1)

    def test_a_repository_with_no_addresses_at_all_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, "no addresses here")
            self.assertEqual(self.run_main(["--root", str(root)], answering(200)), 2)

    def test_the_injected_fetcher_is_the_one_actually_used(self):
        # The defect this guards against is real and was found next door: the
        # fetcher was bound as a default argument, so patching the module did
        # nothing and the tests silently made real network calls.
        calls = []

        def counting(url):
            calls.append(url)
            return vp.Fetched(200, "")

        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, "see https://host-a.co/ok")
            self.run_main(["--root", str(root)], counting)
        self.assertEqual(calls, ["https://host-a.co/ok"], "the stub was never called")

    def test_a_nonsense_cap_or_budget_is_a_usage_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.repo(tmp, "see https://host-a.co/ok")
            self.assertEqual(
                self.run_main(["--root", str(root), "--cap", "0"], answering(200)), 2)
            self.assertEqual(
                self.run_main(["--root", str(root), "--budget", "0"], answering(200)), 2)


class KnownNotLiveTests(unittest.TestCase):
    """The list of addresses known not to answer, and how it expires itself."""

    def setup_repo(self, tmp, body, known_text):
        root = pathlib.Path(tmp) / "repo"
        root.mkdir()
        (root / "README.md").write_text(body, encoding="utf-8")
        known = pathlib.Path(tmp) / "known.toml"
        known.write_text(known_text, encoding="utf-8")
        return root, known

    def run_main(self, argv, fetcher):
        real = links.fetch
        links.fetch = fetcher
        try:
            return links.main(argv)
        finally:
            links.fetch = real

    ENTRY = (
        '[[address]]\nurl = "https://host-a.co/repo"\n'
        'reason = "The repository is private until the flip"\n'
    )

    def test_a_listed_address_that_is_dead_does_not_fail_the_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, known = self.setup_repo(tmp, "see https://host-a.co/repo", self.ENTRY)
            code = self.run_main(
                ["--root", str(root), "--known", str(known)], answering(404))
        self.assertEqual(code, 0)

    def test_a_listed_address_that_starts_answering_fails_and_asks_for_removal(self):
        # The entry has outlived its reason. Left alone it would go on excusing
        # an address that genuinely breaks later, which is how an exemption
        # list becomes a place things hide.
        with tempfile.TemporaryDirectory() as tmp:
            root, known = self.setup_repo(tmp, "see https://host-a.co/repo", self.ENTRY)
            out = pathlib.Path(tmp) / "links.json"
            code = self.run_main(
                ["--root", str(root), "--known", str(known), "--out", str(out)],
                answering(200))
            written = json.loads(out.read_text())
        self.assertEqual(code, 1)
        self.assertEqual(written["results"][0]["verdict"], "no-longer-expected")

    def test_an_unlisted_dead_address_still_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, known = self.setup_repo(tmp, "see https://host-b.co/gone", self.ENTRY)
            code = self.run_main(
                ["--root", str(root), "--known", str(known)], answering(404))
        self.assertEqual(code, 1)

    def test_an_entry_with_no_reason_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, known = self.setup_repo(
                tmp, "see https://host-a.co/repo",
                '[[address]]\nurl = "https://host-a.co/repo"\n')
            with self.assertRaises(SystemExit) as caught:
                self.run_main(
                    ["--root", str(root), "--known", str(known)], answering(404))
        self.assertIn("reason", str(caught.exception))

    def test_a_duplicated_entry_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, known = self.setup_repo(
                tmp, "see https://host-a.co/repo", self.ENTRY + self.ENTRY)
            with self.assertRaises(SystemExit) as caught:
                self.run_main(
                    ["--root", str(root), "--known", str(known)], answering(404))
        self.assertIn("twice", str(caught.exception))

    def test_a_missing_file_simply_means_nothing_is_excused(self):
        self.assertEqual(links.load_known(pathlib.Path("/no/such/known.toml")), {})

    def test_the_shipped_file_parses_and_every_entry_justifies_itself(self):
        here = pathlib.Path(__file__).resolve().parent
        known = links.load_known(here / "prose-links-known.toml")
        self.assertGreaterEqual(len(known), 1)
        for url, reason in known.items():
            self.assertGreater(len(reason), 40, f"{url}'s reason says too little")
            self.assertNotIn("flaky", reason.lower())


class ExtractionOfEscapedProseTests(unittest.TestCase):
    def test_an_escaped_regex_does_not_become_a_request(self):
        # RELEASING.md writes the OIDC subject as a regex, so the address in it
        # is spelled `https://github\.com/...`. Requesting that reached nothing
        # and reported it as a broken link.
        found = links.URL_PATTERN.findall(r"matches https://github\.com/x/y@refs")
        self.assertEqual(found, ["https://github"])

    def test_a_toml_line_continuation_is_not_part_of_the_address(self):
        found = links.URL_PATTERN.findall("url = https://archive.org/details/x\\\n")
        self.assertEqual(found, ["https://archive.org/details/x"])


class ProseFileSelectionTests(unittest.TestCase):
    def test_vendored_packages_are_not_this_project_s_prose(self):
        # The first real run read docs/node_modules and reported eight dead
        # addresses belonging to other people's packages.
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / "docs" / "node_modules" / "pkg").mkdir(parents=True)
            (root / "docs" / "node_modules" / "pkg" / "readme.md").write_text(
                "https://somebody-else.co/broken", encoding="utf-8")
            (root / "docs" / "guide").mkdir(parents=True)
            (root / "docs" / "guide" / "ours.md").write_text(
                "https://ours.co/page", encoding="utf-8")
            # Not a git repository, so this exercises the fallback path.
            found = links.addresses_in(root)
        self.assertIn("https://ours.co/page", found)
        self.assertNotIn("https://somebody-else.co/broken", found)


class ShippedProseTests(unittest.TestCase):
    def test_this_repository_has_prose_with_addresses_in_it(self):
        # A guard on the globs rather than on the network: if somebody renames
        # a directory, this checker would quietly start reading nothing, and a
        # checker that reads nothing is the failure it exists to catch.
        root = pathlib.Path(__file__).resolve().parents[2]
        found = links.addresses_in(root)
        self.assertGreater(len(found), 20, "the prose globs match almost nothing")
        checkable = [u for u in found if links.why_skipped(u) is None]
        self.assertGreater(len(checkable), 10, "everything was skipped as an example")

    def test_the_example_addresses_in_the_guides_are_all_skipped(self):
        root = pathlib.Path(__file__).resolve().parents[2]
        for url in links.addresses_in(root):
            if "172.24.0.2" in url or "10.1.2.3" in url:
                self.assertIsNotNone(
                    links.why_skipped(url),
                    f"{url} would be requested on every nightly run")


if __name__ == "__main__":
    unittest.main()

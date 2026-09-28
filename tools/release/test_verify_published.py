#!/usr/bin/env python3
# Author:  Daniel Iwugo
# Comment: Christ is King
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Daniel Iwugo
"""Tests for the post-publish verifier.

Nothing here touches the network. Every test injects a fetcher, because a test
that needed a live page would be skipped on the machine that most needs to run
it and would fail for reasons that have nothing to do with this code.
"""

import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import verify_published as vp  # noqa: E402


def answering(status=200, body=""):
    """A fetcher that always gives the same answer."""
    return lambda url: vp.Fetched(status, body)


def refusing(exc):
    def fetcher(url):
        raise exc

    return fetcher


class CheckTests(unittest.TestCase):
    def test_a_page_that_says_what_it_should_passes(self):
        channel = {"id": "x", "url": "https://e/{version}", "must_contain": ["v{version}"]}
        row = vp.check(channel, "1.2.3", answering(body="a page mentioning v1.2.3 here"))
        self.assertTrue(row["ok"], row)
        self.assertEqual(row["url"], "https://e/1.2.3")
        self.assertIsNone(row["problem"])

    def test_a_page_missing_the_version_is_the_whole_point(self):
        # The four day Internet Archive failure in one test: the page answers
        # 200, looks fine to a person glancing at it, and states the previous
        # release.
        channel = {"id": "x", "url": "https://e", "must_contain": ["v1.2.3"]}
        row = vp.check(channel, "1.2.3", answering(body="the page still shows v1.2.2"))
        self.assertFalse(row["ok"])
        self.assertIn("v1.2.3", row["problem"])

    def test_every_missing_string_is_named_not_counted(self):
        channel = {"id": "x", "url": "https://e", "must_contain": ["alpha", "beta", "gamma"]}
        row = vp.check(channel, "1.0.0", answering(body="only beta is here"))
        self.assertIn("alpha", row["problem"])
        self.assertIn("gamma", row["problem"])
        self.assertNotIn("beta", row["problem"])

    def test_a_json_path_is_compared_rather_than_searched_for(self):
        # A substring check would pass on a page that merely mentioned the
        # version anywhere, including in a changelog of older releases.
        channel = {
            "id": "x",
            "url": "https://e",
            "json_path": "crate.max_version",
            "must_equal": "{version}",
        }
        body = json.dumps({"crate": {"max_version": "1.2.3", "name": "x"}})
        self.assertTrue(vp.check(channel, "1.2.3", answering(body=body))["ok"])

        stale = json.dumps({"crate": {"max_version": "1.2.2"}})
        row = vp.check(channel, "1.2.3", answering(body=stale))
        self.assertFalse(row["ok"])
        self.assertIn("1.2.2", row["problem"])

    def test_a_missing_json_path_says_where_it_stopped(self):
        channel = {
            "id": "x",
            "url": "https://e",
            "json_path": "crate.max_version",
            "must_equal": "1.0.0",
        }
        row = vp.check(channel, "1.0.0", answering(body=json.dumps({"crate": {}})))
        self.assertFalse(row["ok"])
        self.assertIn("max_version", row["problem"])

    def test_a_page_that_is_not_json_is_reported_as_that(self):
        channel = {"id": "x", "url": "https://e", "json_path": "a", "must_equal": "b"}
        row = vp.check(channel, "1.0.0", answering(body="<html>maintenance</html>"))
        self.assertFalse(row["ok"])
        self.assertIn("did not answer with JSON", row["problem"])

    def test_a_404_is_reported_with_its_status(self):
        channel = {"id": "x", "url": "https://e", "must_contain": ["anything"]}
        row = vp.check(channel, "1.0.0", answering(status=404, body=""))
        self.assertFalse(row["ok"])
        self.assertEqual(row["status"], 404)
        self.assertIn("404", row["problem"])

    def test_an_unreachable_host_is_a_row_rather_than_a_crash(self):
        # One dead channel must not stop the other six being checked, and the
        # run has to produce a record either way.
        channel = {"id": "x", "url": "https://e", "must_contain": ["a"]}
        row = vp.check(channel, "1.0.0", refusing(RuntimeError("no route to host")))
        self.assertFalse(row["ok"])
        self.assertIn("no route to host", row["problem"])


class ChannelFileTests(unittest.TestCase):
    def load(self, text):
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "channels.toml"
            path.write_text(text, encoding="utf-8")
            return vp.load_channels(path)

    def test_the_shipped_channel_file_is_valid(self):
        here = pathlib.Path(__file__).resolve().parent
        channels = vp.load_channels(here / "channels.toml")
        self.assertGreaterEqual(len(channels), 5, "the shipped file lists almost nothing")
        for channel in channels:
            self.assertIn("what", channel, f"{channel['id']} does not say what it is")

    def test_a_channel_with_no_condition_is_refused(self):
        # It would report success for any page that answers 200, including a
        # parked domain, which is worse than not checking it at all.
        with self.assertRaises(SystemExit) as caught:
            self.load('[[channel]]\nid = "x"\nurl = "https://e"\n')
        self.assertIn("states no condition", str(caught.exception))

    def test_a_json_path_with_nothing_to_compare_is_refused(self):
        with self.assertRaises(SystemExit) as caught:
            self.load('[[channel]]\nid = "x"\nurl = "https://e"\njson_path = "a"\n')
        self.assertIn("nothing to", str(caught.exception))

    def test_an_empty_file_is_refused_rather_than_passing(self):
        with self.assertRaises(SystemExit) as caught:
            self.load("# nothing here\n")
        self.assertIn("checked nothing", str(caught.exception))

    def test_two_channels_sharing_an_id_are_refused(self):
        with self.assertRaises(SystemExit) as caught:
            self.load(
                '[[channel]]\nid = "x"\nurl = "https://a"\nmust_contain = ["a"]\n'
                '[[channel]]\nid = "x"\nurl = "https://b"\nmust_contain = ["b"]\n'
            )
        self.assertIn("share the id", str(caught.exception))


class RetryTests(unittest.TestCase):
    def test_only_the_retryable_statuses_are_retried(self):
        # The lesson from this project's own upload helper, which retried its
        # own permanent refusals five times and spent two minutes sleeping
        # before reporting an error it knew at once.
        self.assertIn(503, vp.RETRYABLE_STATUSES)
        self.assertIn(429, vp.RETRYABLE_STATUSES)
        for permanent in (400, 401, 403, 404, 410):
            self.assertNotIn(permanent, vp.RETRYABLE_STATUSES)

    def test_the_read_is_bounded(self):
        # A destination that answers with a gigabyte would otherwise be the
        # verifier's own denial of service.
        self.assertLessEqual(vp.MAX_BODY_BYTES, 16 * 1024 * 1024)
        self.assertGreater(vp.MAX_BODY_BYTES, 64 * 1024)

    def test_every_request_has_a_deadline(self):
        self.assertGreater(vp.TIMEOUT_SECONDS, 0)
        self.assertLessEqual(vp.TIMEOUT_SECONDS, 60)


class MainTests(unittest.TestCase):
    def run_main(self, argv, fetcher):
        real = vp.fetch
        vp.fetch = fetcher
        try:
            return vp.main(argv)
        finally:
            vp.fetch = real

    def channels_file(self, tmp, required):
        path = pathlib.Path(tmp) / "channels.toml"
        path.write_text(
            f'[[channel]]\nid = "one"\nwhat = "a"\nurl = "https://e"\n'
            f'must_contain = ["v{{version}}"]\nrequired = {str(required).lower()}\n',
            encoding="utf-8",
        )
        return path

    def test_a_required_channel_that_is_wrong_fails_the_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = pathlib.Path(tmp) / "distribution.json"
            code = self.run_main(
                [
                    "--version",
                    "1.0.0",
                    "--channels",
                    str(self.channels_file(tmp, True)),
                    "--out",
                    str(out),
                ],
                answering(body="this page has not been updated"),
            )
            self.assertEqual(code, 1)
            # Written even though the check failed: a record of a failed check
            # is more useful than no record, and the exit code carries the
            # verdict.
            written = json.loads(out.read_text())
            self.assertEqual(written["version"], "1.0.0")
            self.assertFalse(written["channels"][0]["ok"])
            self.assertIn("checked_utc", written)

    def test_a_channel_that_is_not_live_yet_does_not_fail_the_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            code = self.run_main(
                ["--version", "1.0.0", "--channels", str(self.channels_file(tmp, False))],
                answering(status=404),
            )
            self.assertEqual(code, 0)

    def test_a_good_run_exits_zero(self):
        with tempfile.TemporaryDirectory() as tmp:
            code = self.run_main(
                ["--version", "1.0.0", "--channels", str(self.channels_file(tmp, True))],
                answering(body="serving v1.0.0 today"),
            )
            self.assertEqual(code, 0)

    def test_the_injected_fetcher_is_the_one_actually_used(self):
        # This test exists because it was not true. `check` bound the real
        # fetcher as a default argument, so replacing it on the module did
        # nothing and three tests made real network calls while appearing to
        # use a stub. Two of them passed for the wrong reason. A suite that
        # reaches the network is one that fails on an aeroplane and passes
        # when the thing it tests is broken.
        calls = []

        def counting(url):
            calls.append(url)
            return vp.Fetched(200, "serving v1.0.0 today")

        with tempfile.TemporaryDirectory() as tmp:
            code = self.run_main(
                ["--version", "1.0.0", "--channels", str(self.channels_file(tmp, True))],
                counting,
            )
        self.assertEqual(code, 0)
        self.assertEqual(calls, ["https://e"], "the stub was never called")

    def test_a_live_channel_that_is_still_optional_fails_the_run(self):
        # `required = false` describes a channel that does not exist yet. Once
        # the page serves the release that reason is spent, and leaving it
        # optional means a later takedown would be reported and pass. The flip
        # is a step in RELEASING.md, which is to say it depended on somebody
        # remembering it.
        with tempfile.TemporaryDirectory() as tmp:
            out = pathlib.Path(tmp) / "distribution.json"
            code = self.run_main(
                [
                    "--version",
                    "1.0.0",
                    "--channels",
                    str(self.channels_file(tmp, False)),
                    "--out",
                    str(out),
                ],
                answering(body="serving v1.0.0 today"),
            )
            self.assertEqual(code, 1)
            written = json.loads(out.read_text())
            row = written["channels"][0]
            self.assertTrue(row["ok"], "the page did state the release")
            self.assertTrue(row["should_be_required"], row)

    def test_a_live_required_channel_is_not_asked_to_flip(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = pathlib.Path(tmp) / "distribution.json"
            code = self.run_main(
                [
                    "--version",
                    "1.0.0",
                    "--channels",
                    str(self.channels_file(tmp, True)),
                    "--out",
                    str(out),
                ],
                answering(body="serving v1.0.0 today"),
            )
            self.assertEqual(code, 0)
            self.assertFalse(json.loads(out.read_text())["channels"][0]["should_be_required"])

    def test_an_optional_channel_that_is_not_live_is_not_asked_to_flip(self):
        row = vp.check(
            {"id": "x", "url": "https://e", "must_contain": ["a"], "required": False},
            "1.0.0",
            answering(status=404),
        )
        self.assertFalse(row["should_be_required"])

    def test_asking_for_a_channel_that_does_not_exist_is_a_usage_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            code = self.run_main(
                [
                    "--version",
                    "1.0.0",
                    "--channels",
                    str(self.channels_file(tmp, True)),
                    "--only",
                    "nope",
                ],
                answering(body="anything"),
            )
            self.assertEqual(code, 2)


if __name__ == "__main__":
    unittest.main()

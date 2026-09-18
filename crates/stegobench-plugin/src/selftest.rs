// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Asking a tool both questions, and believing it only if it gets both right.
//!
//! A tool that answers "stego" to everything passes a detect-only check. One
//! that answers "clean" to everything passes a clear-only check. Either alone
//! is a control that cannot fail. So both fixtures are run and both must come
//! back correct, and a tool that fails either is reported as broken rather
//! than as present.
//!
//! This is the module that turns `doctor`'s "not verified" into an answer.

use std::path::Path;
use std::process::Command;

use stegobench_core::registry::Entry;

use crate::parsers::{self, Reading};

/// The outcome of asking one tool both questions.
#[derive(Debug, Clone, PartialEq)]
pub enum Verified {
    /// Correct on both fixtures.
    Passed,
    /// Wrong on at least one, with which and how.
    Failed(String),
    /// Could not be asked, which is not the same as being wrong.
    Skipped(String),
}

/// Runs one image through a containerised tool and parses what comes back.
///
/// The container is locked down the way every run in this project is: no
/// network, no capabilities, no new privileges, read-only root, and a memory
/// cap. A self-test is still running somebody else's code on our machine.
fn run_one(entry: &Entry, image: &str, fixture: &Path) -> Reading {
    let Some(invoke) = &entry.invoke else {
        return Reading::Failed("entry declares no invoke block".into());
    };
    let Ok(absolute) = fixture.canonicalize() else {
        return Reading::Failed(format!("fixture {} not found", fixture.display()));
    };
    let Some(name) = absolute.file_name().and_then(|n| n.to_str()) else {
        return Reading::Failed("fixture has no usable filename".into());
    };
    let mount = format!("{}:/work/{}:ro", absolute.display(), name);
    let inner = format!("/work/{name}");

    let mut args: Vec<String> = vec![
        "run".into(), "--rm".into(),
        "--network=none".into(),
        "--cap-drop=ALL".into(),
        "--security-opt".into(), "no-new-privileges".into(),
        "--read-only".into(),
        "--memory=2g".into(),
        "-v".into(), mount,
        image.into(),
    ];
    args.extend(
        invoke
            .argv
            .iter()
            .map(|a| a.replace("{file}", &inner)),
    );

    match Command::new("docker").args(&args).output() {
        Ok(out) => parsers::parse(
            &invoke.parser,
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
        ),
        Err(e) => Reading::Failed(format!("could not run the container: {e}")),
    }
}

/// Asks a tool both questions.
///
/// `fixtures_dir` holds the files the entry's selftest block names.
pub fn run(entry: &Entry, fixtures_dir: &Path) -> Verified {
    let Some(test) = &entry.selftest else {
        return Verified::Skipped("no selftest declared".into());
    };
    if entry.invoke.is_none() {
        return Verified::Skipped(
            "no invoke block yet, so this tool cannot be driven by the host".into(),
        );
    }
    let Some(image) = entry.image.as_ref().map(|i| i.reference.clone()) else {
        return Verified::Skipped("self-test currently covers containerised tools".into());
    };

    let detect_path = fixtures_dir.join(strip_prefix(&test.must_detect));
    let clear_path = fixtures_dir.join(strip_prefix(&test.must_clear));

    let threshold = 0.5;
    let higher = entry.emits.higher_means_stego;

    let on_stego = run_one(entry, &image, &detect_path);
    let on_clean = run_one(entry, &image, &clear_path);

    match (
        on_stego.says_stego(higher, threshold),
        on_clean.says_stego(higher, threshold),
    ) {
        (Some(true), Some(false)) => Verified::Passed,
        // Both halves are named separately, because "says yes to everything"
        // and "says no to everything" are different faults with different
        // fixes, and a single "failed" would hide which one it is.
        (Some(true), Some(true)) => Verified::Failed(
            "says stego on the clean fixture too, so it answers yes to everything".into(),
        ),
        (Some(false), Some(false)) => Verified::Failed(
            "says clean on the stego fixture too, so it answers no to everything".into(),
        ),
        (Some(false), Some(true)) => Verified::Failed(
            "has both answers exactly backwards".into(),
        ),
        (None, _) => Verified::Failed(format!("could not read the stego fixture: {on_stego:?}")),
        (_, None) => Verified::Failed(format!("could not read the clean fixture: {on_clean:?}")),
    }
}

/// Registry entries write `fixtures/clean.png`; the directory is passed
/// separately, so the prefix is dropped rather than doubled.
fn strip_prefix(name: &str) -> &str {
    name.strip_prefix("fixtures/").unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(extra: &str) -> Entry {
        toml::from_str(&format!(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"MIT\"\n{extra}\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n"
        ))
        .expect("parses")
    }

    #[test]
    fn a_tool_with_no_invoke_block_is_skipped_not_failed() {
        // Skipped and Failed must stay distinct: "we have not taught the host
        // to drive this yet" is not "this tool is broken".
        let e = entry("[image]\nreference = \"x@sha256:a\"\nsize_mb = 10\nbundled = true");
        assert!(matches!(run(&e, Path::new(".")), Verified::Skipped(_)));
    }

    #[test]
    fn a_binary_tool_is_skipped_with_a_reason() {
        let e = entry(
            "[binary]\ncommand = [\"x\"]\nversion_args = [\"-v\"]\n\
             [invoke]\nargv = [\"x\"]\nparser = \"zsteg\"",
        );
        match run(&e, Path::new(".")) {
            Verified::Skipped(r) => assert!(r.contains("containerised")),
            other => panic!("expected Skipped, got {other:?}"),
        }
    }

    #[test]
    fn the_fixtures_prefix_is_not_doubled() {
        assert_eq!(strip_prefix("fixtures/clean.png"), "clean.png");
        assert_eq!(strip_prefix("clean.png"), "clean.png");
    }

    #[test]
    fn a_missing_fixture_is_a_failure_with_the_path() {
        let e = entry(
            "[image]\nreference = \"x@sha256:a\"\nsize_mb = 10\nbundled = true\n\
             [invoke]\nargv = [\"true\"]\nparser = \"zsteg\"",
        );
        match run(&e, Path::new("/definitely/not/here")) {
            Verified::Failed(r) => assert!(r.contains("not found"), "got {r}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}

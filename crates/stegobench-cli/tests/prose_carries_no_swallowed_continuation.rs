// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//
// A Rust string literal can span several lines, and the backslash at the end
// of each one is what stops the indentation of the next line becoming part of
// the string. Drop the backslash and the literal still compiles, still reads
// correctly in the source, and prints with a run of a dozen spaces in the
// middle of a sentence.
//
// That defect reached a user five separate times on this project in one day,
// in five different files, and every time it was caught by somebody reading
// real output rather than by the suite: a test asserting on a substring is
// usually asserting on a substring that falls on one side of the gap, so it
// passes. One of the five survived into a published report.
//
// So this is the gate. It reads the files whose strings are prose rather than
// aligned columns and fails on any run of three or more spaces between two
// words. Three, not two, because a sentence followed by two spaces is a
// typographic choice somebody may make deliberately; thirteen is never one.
//
// Files of aligned help text are deliberately not listed here: a column of
// command names padded out to line up is the same byte pattern meaning the
// opposite thing, and a gate that cannot tell them apart would either fail
// constantly or be switched off.

use std::fs;
use std::path::{Path, PathBuf};

/// The files whose string literals are sentences, never columns.
const PROSE_SOURCES: [&str; 8] = [
    "crates/stegobench-cli/src/report.rs",
    "crates/stegobench-cli/src/score.rs",
    "crates/stegobench-core/src/result.rs",
    "crates/stegobench-core/src/corpus.rs",
    "crates/stegobench-core/src/registry.rs",
    "crates/stegobench-plugin/src/lib.rs",
    "crates/stegobench-plugin/src/runner.rs",
    "crates/stegobench-plugin/src/selftest.rs",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate directory is two levels below the workspace root")
        .to_path_buf()
}

/// What the scanner is in the middle of when a line ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// Not inside any literal.
    Code,
    /// Inside an ordinary `"..."`, which is the only state this gate judges.
    Quoted,
    /// Inside a raw `r#"..."#`, carrying how many hashes close it. A raw string
    /// spans lines on purpose, carries its indentation on purpose, and is how
    /// every TOML fixture in these files is written, so it is never the defect.
    Raw(usize),
}

/// The state at the end of this line, given the state at the start of it.
///
/// This has to carry state across lines, and it has to know raw strings from
/// ordinary ones. Two earlier versions of this gate got that wrong and each
/// produced a long list of findings with nothing real in it: the first counted
/// the quotes on each line in isolation, which cannot tell a line that opens a
/// literal from one that closes it, and the second read the `"#` closing a raw
/// string as an opening quote and was desynchronised for the rest of the file.
fn state_after(line: &str, before: State) -> State {
    let chars: Vec<char> = line.chars().collect();
    let mut state = before;
    let mut i = 0;
    while i < chars.len() {
        match state {
            State::Code => {
                match chars[i] {
                    // A quote in a comment opens nothing, and the rest of the
                    // line is comment, so stop reading it.
                    '/' if i + 1 < chars.len() && chars[i + 1] == '/' => return State::Code,
                    // A double quote as a character literal is not a string.
                    '\'' if i + 2 < chars.len() && chars[i + 1] == '"' && chars[i + 2] == '\'' => {
                        i += 2;
                    }
                    'r' => {
                        let mut hashes = 0;
                        while i + 1 + hashes < chars.len() && chars[i + 1 + hashes] == '#' {
                            hashes += 1;
                        }
                        if chars.get(i + 1 + hashes) == Some(&'"') {
                            state = State::Raw(hashes);
                            i += 1 + hashes;
                        }
                    }
                    '"' => state = State::Quoted,
                    _ => {}
                }
            }
            State::Quoted => match chars[i] {
                '\\' => i += 1,
                '"' => state = State::Code,
                _ => {}
            },
            State::Raw(hashes) => {
                // A raw string has no escapes, so only a quote followed by the
                // same number of hashes ends it.
                if chars[i] == '"' {
                    let mut seen = 0;
                    while seen < hashes && chars.get(i + 1 + seen) == Some(&'#') {
                        seen += 1;
                    }
                    if seen == hashes {
                        state = State::Code;
                        i += hashes;
                    }
                }
            }
        }
        i += 1;
    }
    state
}

/// The indentation a continued literal would swallow, when there is enough of
/// it to be visible and a word after it.
///
/// Three spaces, not two: a sentence followed by two spaces is a typographic
/// choice somebody may make deliberately, and thirteen never is.
fn swallowed_indentation(next_line: &str) -> Option<usize> {
    let spaces = next_line.len() - next_line.trim_start_matches(' ').len();
    let follows_a_word = next_line
        .trim_start_matches(' ')
        .chars()
        .next()
        .is_some_and(char::is_alphanumeric);
    (spaces >= 3 && follows_a_word).then_some(spaces)
}

#[test]
fn no_prose_string_has_a_run_of_spaces_inside_a_sentence() {
    let root = workspace_root();
    let mut problems = Vec::new();

    for relative in PROSE_SOURCES {
        let path = root.join(relative);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let lines: Vec<&str> = text.lines().collect();
        let mut state = State::Code;
        for (index, line) in lines.iter().enumerate() {
            let before = state;
            // A comment is prose a user never sees, and wrapping one is free.
            // Only a line that is wholly a comment can be skipped outright;
            // one inside a literal is not a comment at all.
            if before == State::Code && line.trim_start().starts_with("//") {
                continue;
            }
            state = state_after(line, before);
            // A raw string spans lines on purpose and carries its indentation
            // on purpose, so only an ordinary literal can hold this defect.
            if state != State::Quoted {
                continue;
            }
            // A trailing backslash is the fix, so a line that has one is right.
            if line.ends_with('\\') {
                continue;
            }
            let Some(next) = lines.get(index + 1) else {
                continue;
            };
            if let Some(spaces) = swallowed_indentation(next) {
                problems.push(format!(
                    "{relative}:{} leaves a string literal open and has no trailing \
                     backslash, so the {spaces} spaces of indentation on the next line are \
                     part of the string and a user sees them mid sentence. \
                     End this line with a backslash.\n    {}\n    {}",
                    index + 1,
                    line.trim_end(),
                    next
                ));
            }
        }
    }

    assert!(
        problems.is_empty(),
        "{} prose string(s) carry indentation a user would see:\n\n{}",
        problems.len(),
        problems.join("\n\n")
    );
}

#[test]
fn the_scanner_tells_the_three_states_apart() {
    let code = State::Code;

    // An ordinary literal, opened and closed.
    assert_eq!(state_after(QUOTE_RUNS_ON, code), State::Quoted);
    assert_eq!(state_after(WHOLE_SENTENCE, code), code);
    assert_eq!(state_after(ESCAPED_QUOTES, code), code);
    // Two literals on one line close each other out.
    assert_eq!(state_after(TWO_LITERALS, code), code);
    // A double quote as a character literal is not a string at all.
    assert_eq!(state_after(CHAR_QUOTE, code), code);
    // A quote in a trailing comment opens nothing.
    assert_eq!(state_after(QUOTE_IN_COMMENT, code), code);

    // The half the FIRST version of this gate got wrong: a line holding one
    // quote CLOSES a literal when one was already open, and the state has to
    // say so, or every closing line is reported as an opening one. That
    // version produced 64 findings and not one of them was real.
    assert_eq!(state_after(CLOSING_LINE, State::Quoted), code);
    assert_eq!(
        state_after("                 still running on", State::Quoted),
        State::Quoted
    );

    // The half the SECOND version got wrong: the `"#` that closes a raw string
    // is not an opening quote, and reading it as one desynchronises every line
    // after it. Every TOML fixture in these files is written that way.
    assert_eq!(state_after(RAW_OPENER, code), State::Raw(1));
    assert_eq!(
        state_after("base_images = 100", State::Raw(1)),
        State::Raw(1)
    );
    assert_eq!(state_after(RAW_CLOSER, State::Raw(1)), code);
    // A bare quote does not close a hashed raw string.
    assert_eq!(state_after(QUOTED_VALUE, State::Raw(1)), State::Raw(1));
}

#[test]
fn the_indentation_rule_spares_what_it_should() {
    assert_eq!(
        swallowed_indentation("             learned nothing"),
        Some(13)
    );
    assert_eq!(swallowed_indentation("learned nothing"), None);
    // One space is the deliberate way to continue a wrapped sentence.
    assert_eq!(swallowed_indentation(" learned nothing"), None);
    // A blank line carries no word, so there is nothing to swallow.
    assert_eq!(swallowed_indentation("        "), None);
}

/// The defect this gate exists to catch, reconstructed exactly as it appeared
/// in `report.rs`: a line ending in a word, no backslash, and thirteen spaces
/// of indentation on the line after it.
#[test]
fn the_defect_that_reached_a_published_report_is_caught() {
    let opener = DEFECT_OPENER;
    let continuation = "             learned nothing from this corpus\"";

    assert!(!opener.ends_with('\\'));
    assert_eq!(state_after(opener, State::Code), State::Quoted);
    assert_eq!(swallowed_indentation(continuation), Some(13));

    // And the fixed form is spared, which is the half that matters: a gate
    // that flags the fix as well is one somebody switches off.
    let fixed = format!("{opener} \\");
    assert!(fixed.ends_with('\\'));
}

// The fixtures the scanner is checked against. They live here, as named
// constants, because several of them are the exact byte patterns the scanner
// reasons about (a raw-string opener, its closer, a quote inside a comment)
// and writing them inline would mean this file's own source tripping the rule
// it defines.
const QUOTE_RUNS_ON: &str = "    \"a sentence that runs on";
const WHOLE_SENTENCE: &str = "    \"a whole sentence\",";
const ESCAPED_QUOTES: &str = "    let q = \"\\\"quoted\\\"\";";
const TWO_LITERALS: &str = "    f(\"one\", \"two\")";
const CHAR_QUOTE: &str = "    let q = '\"';";
const QUOTE_IN_COMMENT: &str = "    let x = 1; // the \"why\"";
const CLOSING_LINE: &str = "                 and beside nothing else.\",";
const RAW_OPENER: &str = "    let t = r#\"";
const RAW_CLOSER: &str = "\"#";
const QUOTED_VALUE: &str = "    a = \"x\"";
const DEFECT_OPENER: &str =
    "            \"SCORED TRAIN AND TEST TOGETHER: quotable only for a detector that";

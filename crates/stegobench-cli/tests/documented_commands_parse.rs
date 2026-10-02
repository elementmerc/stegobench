// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//
// Every `stegobench ...` line the public documentation tells a reader to type
// is parsed against the command tree the binary itself parses against. A line
// the binary would refuse is a defect in the documentation, and this is where
// it gets found.
//
// RAISED BY A JOURNEY, 2026-10-02. The README's first command was
// `stegobench fetch stegobench-starter --tier nano --out ./starter`, and the
// flag was `--dest`. A reader copied it, got `unexpected argument '--out'
// found` and exit 2 as the first thing the tool ever said to them, and had no
// way to tell a broken tool from a stale README. The flag was renamed because
// three other commands already called it `--out`, but the lesson is that
// nothing was checking: a documented command and the parser could disagree
// indefinitely and no test in either half would notice.
//
// This parses rather than runs. Running a documented line would download
// corpora, start containers and write files; parsing catches the whole class
// the journey hit, which is a line the tool rejects before doing anything.

use clap::Parser;
use std::fs;
use std::path::{Path, PathBuf};

/// The documents that tell a reader what to type: the README and every page
/// of the guide.
///
/// DERIVED RATHER THAN LISTED. A typed list of ten filenames had three wrong
/// on the first run, and a wrong name in a list like this does not fail
/// loudly for long: somebody deletes the entry and the page stops being
/// checked. Reading the directory means a page added tomorrow is covered
/// without anybody remembering this file exists.
fn command_sources(root: &Path) -> Vec<PathBuf> {
    let guide = root.join("docs/guide");
    let mut found: Vec<PathBuf> = fs::read_dir(&guide)
        .unwrap_or_else(|e| panic!("{} could not be read: {e}", guide.display()))
        .map(|e| e.expect("a readable directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    assert!(
        found.len() >= 10,
        "only {} guide page(s) were found under {}, so this check would \
         cover almost nothing",
        found.len(),
        guide.display()
    );
    // Sorted so a failure names the same page first on every machine.
    found.sort();
    found.insert(0, root.join("README.md"));
    found
}

/// A line carrying any of these is a shape rather than a command: the reader
/// is expected to put something of their own in place of the word, and the
/// parser has no way to guess what.
///
/// `<name>` and the other value placeholders are deliberately NOT here. Clap
/// takes them as the string they are, so the line still proves that the flags
/// exist and that the subcommand accepts that many arguments, which is the
/// whole of what this test is for.
const PLACEHOLDERS_FOR_THE_COMMAND_ITSELF: &[&str] = &[
    "<command>",
    "<subcommand>",
    "<topic>",
    "<kind>",
    "<any command>",
    "...",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate directory is two levels below the workspace root")
        .to_path_buf()
}

/// Split a documented line into words the way a shell would, well enough for
/// a command somebody is meant to copy.
///
/// Quotes group, and nothing else is interpreted. A line needing more than
/// this is a line this test skips rather than one it guesses at: a splitter
/// that silently mis-splits would report a documentation defect that is really
/// a defect in the splitter, and this project has already spent a night on a
/// gate that measured itself.
fn words(line: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    // `<corpus id>` is one thing the reader replaces, so the space inside it
    // does not separate two arguments.
    let mut placeholder = false;
    for c in line.chars() {
        if quote.is_none() {
            match c {
                '<' => placeholder = true,
                '>' => placeholder = false,
                _ => {}
            }
        }
        if placeholder && c.is_whitespace() {
            word.push(c);
            continue;
        }
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => word.push(c),
            (None, '\'') | (None, '"') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if !word.is_empty() || any {
                    out.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            (None, '\\') => return None,
            (None, c) => word.push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    if !word.is_empty() || any {
        out.push(word);
    }
    Some(out)
}

/// The `stegobench ...` commands in one document.
///
/// FENCED BLOCKS AND INLINE CODE SPANS ONLY, WHICH IS A CORRECTION. Reading
/// every line found thirteen "commands" and not one was real: a sentence
/// saying what stegobench measures parsed as a subcommand called `measures`,
/// and an inline span kept its closing backtick and became a flag called
/// `--help``. A gate whose findings are all its own artefacts gets switched
/// off, so it reads the two places a reader copies from and nowhere else.
fn commands_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut block: Option<Vec<String>> = None;
    for raw in text.lines() {
        if raw.trim_start().starts_with("```") {
            match block.take() {
                Some(lines) => found.extend(commands_in_block(&lines)),
                None => block = Some(Vec::new()),
            }
            continue;
        }
        match &mut block {
            Some(lines) => lines.push(raw.to_string()),
            // Inline spans, each taken whole, so a closing backtick never
            // becomes part of an argument.
            None => {
                for span in raw.split('`').skip(1).step_by(2) {
                    found.extend(segments_of(span));
                }
            }
        }
    }
    found
}

/// The commands in one fenced block, which is not every `stegobench` line in
/// it.
///
/// A BLOCK HOLDS THE OUTPUT AS WELL AS THE COMMAND, and the README's own
/// convention says which is which: a line typed by the reader starts with a
/// `$` prompt. In a block that uses prompts, everything without one is output,
/// and the banner this tool prints opens with the sentence "stegobench
/// measures how good a steganography detector is", which parsed as a
/// subcommand called `measures`. In a block with no prompt anywhere, which is
/// how the guide pages write a bare command, every line is a command.
fn commands_in_block(lines: &[String]) -> Vec<String> {
    let prompted = lines.iter().any(|l| l.trim_start().starts_with("$ "));
    let mut found = Vec::new();
    for line in lines {
        let line = line.trim();
        let typed = match line.strip_prefix("$ ") {
            Some(rest) => rest,
            None if prompted => continue,
            None => line,
        };
        found.extend(segments_of(typed));
    }
    found
}

/// The `stegobench` invocations in one line of a block or one inline span,
/// with any aligned description column dropped.
///
/// A run of three or more spaces is a column rather than an argument: these
/// documents list a command and what it does side by side, and the second half
/// is prose. Three rather than two, for the reason the prose gate gives: two
/// spaces after a full stop is a choice somebody may make deliberately.
fn segments_of(line: &str) -> Vec<String> {
    let command = match line.find("   ") {
        Some(at) => &line[..at],
        None => line,
    };
    // A pipeline is several commands and each half has to parse.
    command.split('|').filter_map(command_in_segment).collect()
}

/// One `stegobench ...` invocation, or `None` where this is not one.
///
/// A fenced block holds a command's OUTPUT as readily as the command, and
/// output that happens to begin with the program's own name is the awkward
/// case: `stegobench 1.0.0` is what `--version` prints. So the word after the
/// name has to look like a subcommand or a flag, which costs nothing and
/// leaves prose and output out.
fn command_in_segment(segment: &str) -> Option<String> {
    let rest = segment.trim().strip_prefix("stegobench ")?;
    // Trailing prose after a command on the same line, which these documents
    // write as a comment.
    let rest = rest.split(" #").next().unwrap_or(rest).trim();
    let first = rest.split_whitespace().next()?;
    let looks_like_a_command = first.starts_with('-')
        || first
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '-' || c == '<' || c == '>');
    looks_like_a_command.then(|| rest.to_string())
}

#[test]
fn every_command_the_docs_tell_a_reader_to_type_parses() {
    let root = workspace_root();
    let mut checked = 0;
    let mut failures: Vec<String> = Vec::new();
    for path in command_sources(&root) {
        let name = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} could not be read: {e}", path.display()));
        for command in commands_in(&text) {
            if PLACEHOLDERS_FOR_THE_COMMAND_ITSELF
                .iter()
                .any(|p| command.contains(p))
            {
                continue;
            }
            let Some(mut argv) = words(&command) else {
                continue;
            };
            argv.insert(0, "stegobench".to_string());
            // A span naming a command rather than invoking it: `stegobench
            // examine` in a sentence is a reference, and the only claim it
            // makes is that the subcommand exists. Asking for its help turns
            // "you left the arguments out" into "this command exists", which
            // is the claim to check.
            if argv.len() == 2 && !argv[1].starts_with('-') {
                argv.push("--help".to_string());
            }
            checked += 1;
            if let Err(e) = stegobench_cli::cli::Cli::try_parse_from(&argv) {
                // Clap reports `--help` and `--version` as errors because
                // they are not a parse failure, and a documented `--help` is
                // the most copied line in any README.
                if matches!(
                    e.kind(),
                    clap::error::ErrorKind::DisplayHelp
                        | clap::error::ErrorKind::DisplayVersion
                        | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                ) {
                    continue;
                }
                let first = e.to_string().lines().next().unwrap_or("").to_string();
                failures.push(format!("{name}: stegobench {command}\n    {first}"));
            }
        }
    }
    // A loop that found no commands passes every assertion inside it by never
    // reaching one, and this file would then go green on a README with its
    // examples deleted.
    assert!(
        checked >= 20,
        "only {checked} documented command(s) were parsed, which is too few \
         to be the real set. Either the extractor stopped matching or the \
         documents moved"
    );
    assert!(
        failures.is_empty(),
        "{} documented command(s) the binary would refuse:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

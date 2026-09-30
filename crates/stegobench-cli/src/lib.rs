// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Library half of the `stegobench` command, split out purely so `build.rs`
//! can import the command definition and generate man pages from the same
//! source the binary parses against. There is no other reason for a lib
//! target here; the behaviour lives in the binary.

/// The registry id of the corpus this binary carries a copy of.
///
/// Here rather than in `main.rs` because both halves need it: the binary
/// writes the corpus out and `needs` has to stop telling the reader that
/// obtaining it is theirs to arrange.
pub const STARTER_ID: &str = "stegobench-starter";

/// A path as somewhere a reader could go, rather than as it was typed.
///
/// `doctor` prints where its registry and its fixtures came from, and a
/// relative answer is not one: `fixtures  fixtures` repeats its own key and
/// tells nobody which directory on this machine was read. Resolved against
/// the working directory without touching the filesystem, so a path that has
/// since been removed still prints as the place it was looked for, and no
/// platform gains a verbatim prefix it did not have.
pub fn resolved_path(path: &std::path::Path) -> std::path::PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// An IO error as a sentence, without the operating system's error number.
///
/// `os error 2` names nothing a reader can act on and reads as a crash rather
/// than an answer, which is the whole of why it never reaches a user here.
///
/// Here rather than in `main.rs` because `report` needs the same rendering:
/// it is the one command whose own error type carried a `std::io::Error`
/// straight into its message, and `stegobench report /tmp/nope` answered with
/// `(os error 2)` while every other command had been saying "nothing is
/// there" for as long as this function has existed.
pub fn plain_io(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "nothing is there".to_string(),
        std::io::ErrorKind::PermissionDenied => "permission was refused".to_string(),
        _ => {
            let text = e.to_string();
            match text.split_once(" (os error") {
                Some((head, _)) => head.to_string(),
                None => text,
            }
        }
    }
}

pub mod cli;
pub mod fetch;
pub mod fixtures;
pub mod help_topics;
pub mod metrics;
pub mod needs;
pub mod registry;
pub mod report;
pub mod score;

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

/// Seconds as something a person can judge a decision against.
///
/// Here rather than in `main.rs` because `plan` and `score` have to agree. One
/// estimates a run before it starts and the other corrects that estimate from
/// what the run then measures, and two renderings of one duration would have
/// the correction read as a disagreement about the format.
pub fn human_duration(seconds: f64) -> String {
    if seconds < 90.0 {
        return format!("{seconds:.0} seconds");
    }
    if seconds < 5_400.0 {
        return format!("{:.0} minutes", seconds / 60.0);
    }
    format!("{:.1} hours", seconds / 3_600.0)
}

/// Why a detector would not be run here, in the words `score` reports it in,
/// or `None` when it would run.
///
/// One function rather than one per command, because `plan` once answered
/// this question by not asking it: a seven detector plan estimated a run that
/// `score` then did with four, overstating the job and saying nothing at all
/// about the three hard blockers. A pre-flight that disagrees with the run it
/// previews is worse than no pre-flight.
///
/// `examine` shares it for the same reason. Asking only whether the code is
/// present would have started a tool it had no command to drive, once per
/// image, and recorded nothing against every one of them.
use std::path::PathBuf;
use stegobench_plugin::availability;
use stegobench_plugin::availability::Presence;

/// PRESENT IS NOT THE SAME AS DRIVABLE, AND `score` USED TO TREAT IT AS THE
/// SAME.
///
/// Availability answers whether the code is on this machine. An entry with no
/// invoke block passes that and still says nothing about what command to
/// launch, so there is nothing to run. `doctor` has reported this since it
/// shipped and `score` did not: the run announced "1 of 1 that can run here",
/// started the tool once per image, recorded "entry declares no invoke block"
/// against every one of them, and then refused with "the corpus holds 0 clean
/// and 0 stego image(s)" over a corpus holding six and twelve. A gap in this
/// project's own registry was reported as a fault in the user's corpus. It is
/// a skip.
///
/// Asked inside the Present arm rather than before the check, so a tool that
/// is neither installed nor drivable is still answered with the half the
/// reader can act on.
pub fn unavailable_reason(
    entry: &stegobench_core::registry::Entry,
    adapter_roots: &[PathBuf],
) -> Option<String> {
    match availability::check(entry, adapter_roots).presence {
        Presence::Present { .. } if entry.invoke.is_none() => Some(
            "declares no invoke block, so nothing in its registry entry says \
             what command to launch and the host has no way to drive it"
                .to_string(),
        ),
        Presence::Present { .. } => None,
        Presence::Unsupported { reason } => Some(format!("cannot run on this machine: {reason}")),
        Presence::Absent { reason } => Some(format!(
            "is registered but is not on this machine: {reason}"
        )),
        Presence::Unknown { reason } => Some(format!(
            "whether it can run here could not be established: {reason}"
        )),
    }
}

/// Image file extensions, lowercased, that a person is likely to have a folder
/// of. Not the set the scorer supports: this is a heuristic for recognising
/// "somebody's pictures", so it is deliberately wider.
///
/// Shared, because two commands now answer a question about the same folder:
/// `score` uses it to recognise that somebody has pointed it at their own
/// pictures, and `examine` uses it to decide which files in a directory it
/// was handed are the images. Two lists would let the refusal and the
/// expansion disagree about what an image is.
pub const LOOKS_LIKE_AN_IMAGE: &[&str] = &[
    "png", "jpg", "jpeg", "tif", "tiff", "bmp", "webp", "gif", "pgm", "ppm", "heic",
];

pub mod cli;
pub mod examine;
pub mod fetch;
pub mod fixtures;
pub mod help_topics;
pub mod metrics;
pub mod needs;
pub mod registry;
pub mod report;
pub mod score;

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
/// ASKED BEFORE THE PRESENCE CHECK RATHER THAN INSIDE THE PRESENT ARM, WHICH
/// IS A CHANGE FROM WHAT THIS COMMENT USED TO ARGUE. The old order answered an
/// entry that was neither installed nor drivable with the install step, on the
/// grounds that it was the half the reader could act on. It is not: installing
/// a tool whose entry says nothing about what command to launch buys a reader
/// a download and the same refusal afterwards, and for one registered entry
/// that download is nine gigabytes. Drivability is a property of the entry, so
/// it is terminal, and `needs::of_tool` decides it in the same position for
/// the same reason. One rule, asked one way, because two commands reading one
/// registry and disagreeing about whether an entry is usable is the defect a
/// journey found here.
pub fn unavailable_reason(
    entry: &stegobench_core::registry::Entry,
    adapter_roots: &[PathBuf],
) -> Option<String> {
    if !crate::needs::can_be_driven(entry) {
        return Some(crate::needs::undrivable_because(entry));
    }
    match availability::check(entry, adapter_roots).presence {
        Presence::Present { .. } => None,
        Presence::Unsupported { reason } => Some(format!("cannot run on this machine: {reason}")),
        Presence::Absent { reason } => Some(format!(
            "is registered but is not on this machine: {}",
            plainer(&reason)
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

/// The cheapest concrete way to get one working detector, or `None` when the
/// registry offers no answer.
///
/// RAISED BY A JOURNEY, 2026-10-02, AND IT WAS THE WHOLE JOURNEY.
///
/// A reader who had never heard the word steganography found `examine` in
/// seconds, understood what the answer was worth in five minutes, and then
/// could not run anything, because nothing is installed on a fresh machine
/// and nothing anywhere said which of seven detectors to install or that one
/// would be enough. They picked one by guessing. `doctor` lists every missing
/// tool in registry order, which puts a 9 GB image above the 287 MB one, and
/// they said they would have given up at the 9 GB line.
///
/// So this answers "which one, and what do I type", and it is derived from
/// the registry rather than written down: a hard coded recommendation would
/// name a tool somebody later removes, and a hard coded size would drift from
/// the entry that declares it. Smallest first, because the barrier a beginner
/// actually hits is the download.
pub fn smallest_way_in(reg: &stegobench_core::registry::Registry) -> Option<String> {
    use stegobench_core::registry::Kind;

    let mut candidates: Vec<(u64, &str, &str)> = reg
        .entries
        .values()
        .filter(|e| e.kind == Kind::Detector)
        .filter_map(|e| {
            let image = e.image.as_ref()?;
            // Only an entry that declares its size can be called the
            // smallest, and only a pinned reference is a thing to recommend
            // pulling. Both are already required of every entry here; the
            // filter is so that a future entry missing one is skipped rather
            // than recommended on a guess.
            Some((image.size_mb?, e.name.as_str(), image.reference.as_str()))
        })
        .collect();
    // Sorted by size, then by name, so two runs over one registry recommend
    // the same tool rather than whichever the map iterated first.
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)));

    let (mb, name, reference) = candidates.first()?;
    Some(format!(
        "To ask anything about an image you need ONE detector, not all of \
         them. The smallest is {name}, about {mb} MB:\n  \
         docker pull {reference}\n  \
         then: stegobench examine <your image> --detector {name}"
    ))
}

/// Any `@sha256:<64 hex>` in a sentence, cut to its first eight characters.
///
/// For PROSE only. A journey called the unavailable line "a wall of hash":
/// the full digest sits in the middle of a sentence a beginner is trying to
/// read, and eight characters is enough to recognise it by. The pasteable
/// command keeps its full digest, because a truncated pull command is worse
/// than a long one.
pub fn shorten_digests(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("@sha256:") {
        let (before, from) = rest.split_at(at);
        out.push_str(before);
        let digest: String = from["@sha256:".len()..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if digest.len() == 64 {
            out.push_str("@sha256:");
            out.push_str(&digest[..8]);
            out.push_str("...");
        } else {
            // Not a full digest, so not the thing this is for. Left exactly
            // as it was rather than half shortened.
            out.push_str("@sha256:");
            out.push_str(&digest);
        }
        rest = &from["@sha256:".len() + digest.len()..];
    }
    out.push_str(rest);
    out
}

/// The same reason, with the two phrases a journey flagged as jargon put in
/// words somebody who has never configured a shell can act on.
///
/// Only the wording changes. "not on PATH" is a true statement about the
/// reader's environment and means nothing to somebody who does not know they
/// have one, and it was shown to exactly that reader on 2026-10-02.
fn plainer(reason: &str) -> String {
    if let Some(program) = reason.strip_suffix(" is not on PATH") {
        return format!(
            "the program `{program}` is not installed, or is not anywhere \
             this can find it"
        );
    }
    reason.to_string()
}

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

#[cfg(test)]
mod novice_findings {
    use super::*;

    /// The recommendation is derived from the registry, so it cannot name a
    /// tool somebody removed or a size that drifted from the entry.
    #[test]
    fn the_smallest_detector_is_the_one_recommended() {
        let reg =
            stegobench_core::registry::Registry::load(std::path::Path::new("plugins/registry"))
                .or_else(|_| {
                    stegobench_core::registry::Registry::load(std::path::Path::new(
                        "../../plugins/registry",
                    ))
                })
                .expect("the shipped registry loads");

        let way = smallest_way_in(&reg).expect("the shipped registry has a sized detector");

        // Whatever it names must actually be the smallest sized detector in
        // the registry, computed here independently of the function.
        use stegobench_core::registry::Kind;
        let mut sizes: Vec<(u64, &str)> = reg
            .entries
            .values()
            .filter(|e| e.kind == Kind::Detector)
            .filter_map(|e| Some((e.image.as_ref()?.size_mb?, e.name.as_str())))
            .collect();
        sizes.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)));
        let (mb, name) = sizes[0];
        assert!(
            way.contains(name),
            "names something other than {name}: {way}"
        );
        assert!(way.contains(&mb.to_string()), "does not say {mb} MB: {way}");

        // And it has to be actionable, which was the whole finding: the
        // reader had to guess both which tool and that a download was the
        // problem.
        assert!(way.contains("docker pull"), "{way}");
        assert!(way.contains("--detector"), "{way}");
        assert!(way.contains("ONE detector"), "{way}");
    }

    #[test]
    fn a_registry_with_no_sized_detector_recommends_nothing_rather_than_guessing() {
        let empty = stegobench_core::registry::Registry::default();
        assert!(smallest_way_in(&empty).is_none());
    }

    #[test]
    fn a_full_digest_in_prose_is_cut_and_a_short_one_is_left_alone() {
        let full = format!("docker pull repo/name@sha256:{}", "a".repeat(64));
        let cut = shorten_digests(&full);
        assert!(cut.ends_with("@sha256:aaaaaaaa..."), "{cut}");
        assert!(!cut.contains(&"a".repeat(64)));

        // Not a full digest, so not the thing this is for: left exactly as
        // it was rather than half shortened.
        let short = "repo/name@sha256:abc";
        assert_eq!(shorten_digests(short), short);

        // Nothing to do.
        assert_eq!(shorten_digests("no digest here"), "no digest here");

        // Two of them in one sentence, and the text around them survives.
        let two = format!(
            "first a@sha256:{} then b@sha256:{} end",
            "b".repeat(64),
            "c".repeat(64)
        );
        let cut = shorten_digests(&two);
        assert_eq!(
            cut, "first a@sha256:bbbbbbbb... then b@sha256:cccccccc... end",
            "{cut}"
        );
    }

    #[test]
    fn the_path_jargon_becomes_something_a_beginner_can_act_on() {
        let plain = plainer("stegcore is not on PATH");
        assert!(plain.contains("`stegcore`"), "{plain}");
        assert!(plain.contains("not installed"), "{plain}");
        assert!(
            !plain.contains("PATH"),
            "still names the reader's shell: {plain}"
        );

        // Any other reason is passed through untouched; only the one phrase a
        // journey flagged is rewritten.
        assert_eq!(
            plainer("not pulled. docker pull x"),
            "not pulled. docker pull x"
        );
    }
}

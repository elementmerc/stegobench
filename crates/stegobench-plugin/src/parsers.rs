// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Reading what each classic tool actually prints.
//!
//! WHY THIS IS ITS OWN MODULE WITH ITS OWN FIXTURES
//! ------------------------------------------------
//! The zsteg parser had three separate bugs in one week while it lived inside
//! a 501 line orchestrator: it matched the wrong marker, then read the wrong
//! stream, then got an operator precedence wrong. Each was hard to see for the
//! same reason, that a parser buried in a big function has no fixtures of its
//! own and is only ever exercised by a twenty minute run.
//!
//! Every parser here is a pure function from captured output to an answer, and
//! every known-bad case that has actually bitten us is a test below.

use crate::Record;

/// What a parser concluded about one image.
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    Score(f64),
    Verdict(bool),
    Failed(String),
}

/// zsteg: a structural scanner that reads the container rather than the pixels.
///
/// THREE THINGS THAT ARE NOT OBVIOUS AND EACH COST A BUG
///
/// 1. A finding is marked `[?]`, not `[+]` or anything else.
/// 2. On a JPEG, zsteg writes **nothing to stdout**: both the finding and the
///    crash that follows go to stderr. Reading only stdout sees a clean image
///    every time, which is a detector that always says no.
/// 3. `text:` and `file:` lines are findings too, and the check for them has
///    to be grouped correctly or it collapses into something always true.
///
/// So both streams are searched, and the caller passes them joined.
pub fn zsteg(stdout: &str, stderr: &str) -> Reading {
    let combined = format!("{stdout}\n{stderr}");
    for line in combined.lines() {
        let line = line.trim();
        if line.starts_with("[?]") {
            return Reading::Verdict(true);
        }
        // Grouped deliberately. Written as `a || b && c` this reads as
        // `a || (b && c)` and quietly stops testing what it looks like it
        // tests, which is the third bug this comment exists to prevent.
        if (line.contains("text:") || line.contains("file:")) && !line.contains("nothing :(") {
            return Reading::Verdict(true);
        }
    }
    // An empty result is a real answer for zsteg: it looked and found nothing.
    Reading::Verdict(false)
}

/// StegExpose prints a CSV whose last column is its fused score.
pub fn stegexpose(stdout: &str, _stderr: &str) -> Reading {
    for line in stdout.lines().rev() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("File name") {
            continue;
        }
        if let Some(last) = line.rsplit(',').next() {
            if let Ok(v) = last.trim().parse::<f64>() {
                return Reading::Score(v);
            }
        }
    }
    Reading::Failed("no numeric score in StegExpose output".into())
}

/// One bare number on stdout, which is what an adapter prints.
///
/// Anything else is a failure rather than a zero. A zero here would be
/// indistinguishable from a confident "clean", which is how a broken tool
/// starts looking like a working one.
pub fn number(stdout: &str, stderr: &str) -> Reading {
    for line in stdout.lines() {
        if let Ok(v) = line.trim().parse::<f64>() {
            if v.is_finite() {
                return Reading::Score(v);
            }
        }
    }
    let why = stderr.lines().next_back().unwrap_or("no output").trim();
    Reading::Failed(format!("no number on stdout: {why}"))
}

/// Stegcore prints a JSON envelope; the score is in the first data record.
///
/// The envelope, not just the number, because `ok: false` is a real outcome
/// and reading a missing score as zero would turn a refusal into a confident
/// "clean".
pub fn stegcore(stdout: &str, stderr: &str) -> Reading {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(stdout) else {
        let why = stderr.lines().next_back().unwrap_or("no output").trim();
        return Reading::Failed(format!("stegcore printed no JSON: {why}"));
    };
    if v.get("ok").and_then(|o| o.as_bool()) == Some(false) {
        let why = v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unspecified");
        return Reading::Failed(format!("stegcore reported failure: {why}"));
    }
    let first = v
        .get("data")
        .and_then(|d| d.as_array().and_then(|a| a.first()).or(Some(d)));
    match first
        .and_then(|r| r.get("overall_score"))
        .and_then(|s| s.as_f64())
    {
        Some(s) => Reading::Score(s),
        None => Reading::Failed("no overall_score in stegcore output".into()),
    }
}

/// Dispatches by the name a registry entry declares.
pub fn parse(parser: &str, stdout: &str, stderr: &str) -> Reading {
    match parser {
        "zsteg" => zsteg(stdout, stderr),
        "stegexpose" => stegexpose(stdout, stderr),
        "number" => number(stdout, stderr),
        "stegcore" => stegcore(stdout, stderr),
        other => Reading::Failed(format!("no built-in parser named {other:?}")),
    }
}

impl Reading {
    /// Turns a reading into a protocol record.
    pub fn into_record(self, id: impl Into<String>) -> Record {
        let id = id.into();
        match self {
            Reading::Score(s) => Record {
                id,
                score: Some(s),
                verdict: None,
                error: None,
                elapsed_ms: None,
            },
            Reading::Verdict(v) => Record {
                id,
                score: None,
                verdict: Some(v),
                error: None,
                elapsed_ms: None,
            },
            Reading::Failed(e) => Record {
                id,
                score: None,
                verdict: None,
                error: Some(e),
                elapsed_ms: None,
            },
        }
    }

    /// Whether this reading means "carrying something", for a self-test.
    /// A failure is not a yes and not a no; it is neither.
    pub fn says_stego(&self, higher_means_stego: bool, threshold: f64) -> Option<bool> {
        match self {
            Reading::Verdict(v) => Some(*v),
            Reading::Score(s) => Some(if higher_means_stego {
                *s > threshold
            } else {
                *s < threshold
            }),
            Reading::Failed(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zsteg_finds_the_question_mark_marker() {
        assert_eq!(
            zsteg("[?] 4096 bytes of extra data\n", ""),
            Reading::Verdict(true)
        );
    }

    #[test]
    fn zsteg_reads_stderr_because_on_jpeg_stdout_is_empty() {
        // Bug two, pinned. Reading only stdout made zsteg a detector that
        // answered "clean" to every JPEG, which looked like a real result.
        assert_eq!(
            zsteg(
                "",
                "[?] 4096 bytes of extra data after IEND\nerror: not a PNG"
            ),
            Reading::Verdict(true)
        );
    }

    #[test]
    fn zsteg_treats_a_text_finding_as_a_finding() {
        assert_eq!(
            zsteg("b1,rgb,lsb,xy .. text: \"hello\"\n", ""),
            Reading::Verdict(true)
        );
    }

    #[test]
    fn zsteg_does_not_fire_on_its_own_nothing_found_line() {
        // Bug three's shape: with the condition grouped wrongly, a line
        // saying it found nothing still matched.
        assert_eq!(
            zsteg("b1,rgb,lsb,xy .. text: nothing :(\n", ""),
            Reading::Verdict(false)
        );
    }

    #[test]
    fn zsteg_on_a_clean_image_says_no_rather_than_failing() {
        // An empty result is an answer, not an error. Reporting it as an
        // error would inflate n_error and shrink the clean set.
        assert_eq!(zsteg("", ""), Reading::Verdict(false));
    }

    #[test]
    fn zsteg_ignores_a_plus_marker_which_is_not_what_it_prints() {
        // Bug one: the original parser looked for [+] and so matched nothing,
        // ever, and reported a 0% detection rate that read as a finding.
        assert_eq!(
            zsteg("[+] something unrelated\n", ""),
            Reading::Verdict(false)
        );
    }

    #[test]
    fn stegexpose_takes_the_last_column_of_the_last_row() {
        let out = "File name,Secret size,Primary Sets,Chi Square,Sample Pairs,RS analysis,Fusion (mean)\n\
                   a.png,1024,0.1,0.2,0.3,0.4,0.2751\n";
        assert_eq!(stegexpose(out, ""), Reading::Score(0.2751));
    }

    #[test]
    fn stegexpose_without_a_number_fails_rather_than_guessing_zero() {
        // A zero here would be indistinguishable from a confident "clean",
        // which is how a broken tool starts looking like a working one.
        assert!(matches!(
            stegexpose("File name,Fusion (mean)\n", ""),
            Reading::Failed(_)
        ));
    }

    #[test]
    fn a_bare_number_is_read_as_a_score() {
        assert_eq!(number("0.0412345678\n", ""), Reading::Score(0.0412345678));
    }

    #[test]
    fn a_negative_estimate_is_kept_because_it_is_the_noise_floor() {
        // Both Aletheia estimators go slightly negative on some clean images.
        // That spread is what a false-positive rate is measured from, so
        // clamping it would make every clean image look identical.
        assert_eq!(number("-0.0093\n", ""), Reading::Score(-0.0093));
    }

    #[test]
    fn no_number_reports_why_rather_than_returning_zero() {
        let r = number(
            "",
            "aletheia is not importable in this container: no module",
        );
        match r {
            Reading::Failed(why) => assert!(why.contains("not importable"), "got {why}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_non_finite_estimate_is_not_a_score() {
        assert!(matches!(number("NaN\n", "bad"), Reading::Failed(_)));
    }

    #[test]
    fn stegcore_reads_the_score_out_of_its_envelope() {
        let out = r#"{"ok":true,"data":[{"overall_score":0.4192,"verdict":"suspicious"}]}"#;
        assert_eq!(stegcore(out, ""), Reading::Score(0.4192));
    }

    #[test]
    fn a_stegcore_failure_is_not_read_as_a_clean_image() {
        // ok:false with no score would parse to zero under a naive reader,
        // turning a refusal into the most confident possible "clean".
        let out = r#"{"ok":false,"error":"unsupported format"}"#;
        match stegcore(out, "") {
            Reading::Failed(why) => assert!(why.contains("unsupported format")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn stegcore_without_json_says_so_rather_than_scoring_zero() {
        assert!(matches!(stegcore("not json", "boom"), Reading::Failed(_)));
    }

    #[test]
    fn an_unknown_parser_name_fails_loudly() {
        assert!(matches!(parse("nope", "", ""), Reading::Failed(_)));
    }

    #[test]
    fn a_failed_reading_is_neither_yes_nor_no() {
        assert_eq!(Reading::Failed("x".into()).says_stego(true, 0.5), None);
    }

    #[test]
    fn a_score_is_compared_in_the_declared_direction() {
        assert_eq!(Reading::Score(0.9).says_stego(true, 0.5), Some(true));
        // outguess inverts StegaShield, so direction is not decoration.
        assert_eq!(Reading::Score(0.9).says_stego(false, 0.5), Some(false));
    }

    #[test]
    fn a_reading_becomes_a_complete_record() {
        assert!(Reading::Verdict(false).into_record("1").is_complete());
        assert!(Reading::Failed("x".into()).into_record("1").is_complete());
    }

    /// The drift guard for the list a registry entry is validated against.
    ///
    /// `stegobench_core::registry::PARSERS` is what refuses a typo in
    /// `invoke.parser` at load, and it is a second copy of the names this
    /// file dispatches on, because core is the layer below this one and
    /// cannot see these functions. A parser added here and not added there
    /// would be refused by the loader although it works; a name removed here
    /// and left there would be accepted by the loader and fail per image,
    /// which is exactly the behaviour the validation was added to stop.
    #[test]
    fn every_name_the_registry_accepts_is_a_parser_this_file_dispatches() {
        for name in stegobench_core::registry::PARSERS {
            let reading = parse(name, "", "");
            if let Reading::Failed(why) = &reading {
                assert!(
                    !why.contains("no built-in parser named"),
                    "{name} is accepted by the registry and dispatches to nothing"
                );
            }
        }

        // And the arm that refuses is still reachable, so the loop above is
        // not passing because everything is accepted.
        let invented = parse("not-a-parser-xyzzy", "", "");
        match invented {
            Reading::Failed(why) => assert!(
                why.contains("no built-in parser named"),
                "an unknown parser should say so: {why}"
            ),
            other => panic!("an unknown parser was handled: {other:?}"),
        }
    }
}

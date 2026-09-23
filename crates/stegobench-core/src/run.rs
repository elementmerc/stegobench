// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! `run-v1`: a set of measurements produced together.
//!
//! A campaign over many arms produces many `result-v1` documents plus one
//! `run-v1` that ties them together: the command line that started it, the
//! registry revision it ran against, the governor's pre-flight estimate next
//! to what the run actually cost, and any arm that failed rather than only the
//! ones that finished. It is what `stegobench watch` reads and what `--resume`
//! restarts from, so it is written incrementally rather than only at the end.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The value of the `schema` field. Readers match on this exactly.
pub const RUN_SCHEMA_ID: &str = "stegobench/run-v1";

/// One campaign, tying together the `result-v1` documents it produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RunV1 {
    /// Always [`RUN_SCHEMA_ID`].
    pub schema: String,
    /// A stable identifier for this run, so `watch` and `--resume` can find it
    /// again without depending on the order arms happen to appear in.
    pub run_id: String,
    /// The command line as typed, so a run can be re-issued exactly.
    pub command: Vec<String>,
    /// A hash or tag identifying the registry contents this run used. Two runs
    /// against different registry revisions are not the same experiment even
    /// when the command line is identical.
    pub registry_revision: String,
    pub started_utc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_utc: Option<String>,
    /// Every arm this run covers, in the order the plan produced them.
    pub arms: Vec<ArmStatus>,
    /// The governor's pre-flight estimate against what happened, so the
    /// estimator can be recalibrated from real runs rather than from guesses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Estimate>,
}

/// One arm's progress within a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArmStatus {
    pub name: String,
    pub state: ArmState,
    /// Path or URI to the `result-v1` document this arm produced, once done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Why the arm failed. Only meaningful when `state` is `Failed`, and
    /// required in that case so a failure is never silent in the run record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ArmState {
    Queued,
    Running,
    Done,
    Failed,
}

/// The governor's pre-flight estimate next to what actually happened.
///
/// This is the calibration feed: on 2026-09-17 a run became 128 Octave
/// workers on 16 cores and produced 298 OOM kills for a measured 1.2x
/// speedup, and nothing in the tooling could have said so in advance because
/// nothing recorded what earlier runs actually cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Estimate {
    pub estimated_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_peak_rss_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_peak_rss_mb: Option<u64>,
}

impl RunV1 {
    /// Checks the things the type system cannot.
    pub fn validate(&self) -> std::result::Result<(), Vec<String>> {
        let mut bad = Vec::new();
        if self.schema != RUN_SCHEMA_ID {
            bad.push(format!(
                "schema is {:?}, expected {RUN_SCHEMA_ID:?}",
                self.schema
            ));
        }
        if self.run_id.trim().is_empty() {
            bad.push(
                "run_id is empty; set it to something stable and unique, such as \
                 the start timestamp plus the arm name, so `watch` and `--resume` \
                 can find this run again"
                    .into(),
            );
        }
        if self.command.is_empty() {
            bad.push(
                "command is empty; set it to the argv the run was actually \
                 launched with, so the run can be re-issued exactly"
                    .into(),
            );
        }
        for arm in &self.arms {
            if arm.state == ArmState::Failed && arm.error.is_none() {
                bad.push(format!(
                    "arm {:?} is failed but names no error; a silent failure in \
                     the run record is exactly what this field exists to prevent",
                    arm.name
                ));
            }
            if arm.state == ArmState::Done && arm.result.is_none() {
                bad.push(format!(
                    "arm {:?} is done but names no result document",
                    arm.name
                ));
            }
        }
        if bad.is_empty() {
            Ok(())
        } else {
            Err(bad)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RunV1 {
        RunV1 {
            schema: RUN_SCHEMA_ID.into(),
            run_id: "2026-09-21T09:00:00Z-suniward".into(),
            command: vec![
                "stegobench".into(),
                "score".into(),
                "--corpus".into(),
                "pentimento-core".into(),
            ],
            registry_revision: "sha256:abc123".into(),
            started_utc: "2026-09-21T09:00:00Z".into(),
            finished_utc: None,
            arms: vec![ArmStatus {
                name: "suniward/0.40".into(),
                state: ArmState::Running,
                result: None,
                error: None,
            }],
            estimate: Some(Estimate {
                estimated_seconds: 3600.0,
                actual_seconds: None,
                estimated_peak_rss_mb: Some(6000),
                actual_peak_rss_mb: None,
            }),
        }
    }

    #[test]
    fn a_well_formed_run_validates() {
        assert_eq!(sample().validate(), Ok(()));
    }

    #[test]
    fn round_trips_through_json_unchanged() {
        let a = sample();
        let text = serde_json::to_string(&a).unwrap();
        let b: RunV1 = serde_json::from_str(&text).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_failed_arm_without_a_reason_is_refused() {
        let mut r = sample();
        r.arms[0].state = ArmState::Failed;
        r.arms[0].error = None;
        assert!(r.validate().unwrap_err()[0].contains("names no error"));
    }

    #[test]
    fn a_failed_arm_with_a_reason_validates() {
        let mut r = sample();
        r.arms[0].state = ArmState::Failed;
        r.arms[0].error = Some("plugin timed out".into());
        assert_eq!(r.validate(), Ok(()));
    }

    #[test]
    fn a_done_arm_without_a_result_is_refused() {
        let mut r = sample();
        r.arms[0].state = ArmState::Done;
        assert!(r.validate().unwrap_err()[0].contains("names no result"));
    }

    #[test]
    fn an_empty_command_is_refused() {
        let mut r = sample();
        r.command.clear();
        assert!(r.validate().unwrap_err()[0].contains("command is empty"));
    }

    #[test]
    fn the_wrong_schema_id_is_refused() {
        let mut r = sample();
        r.schema = "stegobench/run-v2".into();
        assert!(r.validate().unwrap_err()[0].contains("expected"));
    }
}
